//! Abrupt completion and parser control regions.

use crate::engine::api::error::Error;
use crate::engine::api::error::ErrorKind;
use crate::engine::code::bytecode::Instruction;
use crate::engine::code::function::metadata::FunctionKind as BytecodeFunctionKind;
use crate::engine::compiler::lexer::Span;
use crate::engine::compiler::lexer::TokenKind;
use crate::engine::compiler::model::ir::IrConstant;
use crate::engine::compiler::model::scope::ScopeId;
use crate::engine::compiler::parser::context::BreakControlContext;
use crate::engine::compiler::parser::context::BreakControlKind;
use crate::engine::compiler::parser::context::Parser;
use crate::engine::compiler::parser::diagnostics::source_offset;
use crate::engine::value::JsString;
use crate::engine::value::PrimitiveValue as Value;

impl<'source> Parser<'source> {
    pub(in crate::engine::compiler) fn parse_loop_jump_statement(
        &mut self,
        is_continue: bool,
    ) -> Result<(), Error> {
        self.advance()?;

        let label_name = if self.current().line_terminator_before {
            None
        } else if let TokenKind::Identifier(identifier) = &self.current().kind
            && !identifier.escaped_reserved_word
        {
            Some(identifier.value.clone())
        } else {
            None
        };
        let target = self
            .current_ir()
            .context
            .break_controls
            .iter()
            .rposition(|control| match label_name.as_deref() {
                Some(label_name) if is_continue => {
                    matches!(
                        control.kind,
                        BreakControlKind::Loop | BreakControlKind::ForIn | BreakControlKind::ForOf
                    ) && control.label_name.as_deref() == Some(label_name)
                }
                Some(label_name) => control.label_name.as_deref() == Some(label_name),
                None if is_continue => {
                    matches!(
                        control.kind,
                        BreakControlKind::Loop | BreakControlKind::ForIn | BreakControlKind::ForOf
                    )
                }
                None => matches!(
                    control.kind,
                    BreakControlKind::Loop
                        | BreakControlKind::ForIn
                        | BreakControlKind::ForOf
                        | BreakControlKind::Switch
                ),
            });
        let Some(target) = target else {
            return Err(self.syntax_here(if label_name.is_some() {
                "break/continue label not found"
            } else if is_continue {
                "continue must be inside loop"
            } else {
                "break must be inside loop or switch"
            }));
        };
        let source_depth = self.current_ir().context.stack_depth;
        let current_scope = self.current_ir().context.current_scope;
        let (target_scope, entry_depth, crossed_controls) = {
            let controls = &self.current_ir().context.break_controls;
            let target_control = &controls[target];
            let crossed_controls = controls[target + 1..]
                .iter()
                .enumerate()
                .rev()
                .map(|(offset, control)| {
                    (
                        target + 1 + offset,
                        control.kind,
                        control.scope,
                        control.drop_count,
                    )
                })
                .collect::<Vec<_>>();
            (
                target_control.scope,
                target_control.entry_depth,
                crossed_controls,
            )
        };
        let mut cleanup_scope = current_scope;
        for (control_index, control_kind, control_scope, drop_count) in crossed_controls {
            self.emit_scope_closures(cleanup_scope, control_scope)?;
            cleanup_scope = control_scope;
            match control_kind {
                BreakControlKind::TryFinally => {
                    if drop_count != 1 {
                        return Err(Error::internal(
                            "try/finally control has the wrong catch-marker depth",
                        ));
                    }
                    self.emit_instruction(Instruction::DropCatch)?;
                    self.emit_instruction(Instruction::Undefined)?;
                    let gosub = self.emit_instruction(Instruction::Gosub(u32::MAX))?;
                    self.current_ir_mut().context.break_controls[control_index]
                        .finally_gosubs
                        .push(gosub);
                    self.emit_instruction(Instruction::Drop)?;
                }
                BreakControlKind::FinallyBody => {
                    if drop_count != 2 {
                        return Err(Error::internal(
                            "finally-body control has the wrong cleanup depth",
                        ));
                    }
                    // The typed return-address value is at TOS and must never
                    // pass through the ordinary JavaScript-value Drop path.
                    self.emit_instruction(Instruction::DropGosub)?;
                    self.emit_instruction(Instruction::Drop)?;
                }
                BreakControlKind::ForOf => {
                    if drop_count != 3 {
                        return Err(Error::internal(
                            "for-of control has the wrong iterator-record depth",
                        ));
                    }
                    self.emit_instruction(Instruction::IteratorClose)?;
                }
                BreakControlKind::DestructuringIterator => {
                    if drop_count != 3 {
                        return Err(Error::internal(
                            "destructuring control has the wrong iterator-record depth",
                        ));
                    }
                    self.emit_instruction(Instruction::IteratorClose)?;
                }
                BreakControlKind::ForOfAssignmentFragment => {
                    return Err(Error::internal(
                        "break/continue crossed a for-of assignment fragment",
                    ));
                }
                BreakControlKind::RegularStatement
                | BreakControlKind::Loop
                | BreakControlKind::ForIn
                | BreakControlKind::Switch => {
                    for _ in 0..drop_count {
                        self.emit_instruction(Instruction::Drop)?;
                    }
                }
            }
        }
        self.emit_scope_closures(cleanup_scope, target_scope)?;
        self.require_stack_depth(entry_depth, "break/continue cleanup")?;
        let jump = self.emit_instruction(Instruction::Goto(u32::MAX))?;
        let control = self
            .current_ir_mut()
            .context
            .break_controls
            .get_mut(target)
            .ok_or_else(|| Error::internal("break control disappeared while emitting jump"))?;
        if is_continue {
            control.continue_jumps.push(jump);
        } else {
            control.break_jumps.push(jump);
        }
        if label_name.is_some() {
            self.advance()?;
        }
        self.consume_statement_terminator()?;
        // The emitted jump is terminal, but parsing continues linearly so a
        // later case body or ordinary unreachable statement must retain the
        // enclosing control's fallthrough stack shape.
        self.current_ir_mut().context.stack_depth = source_depth;
        Ok(())
    }

    pub(in crate::engine::compiler) fn push_loop_control(
        &mut self,
        entry_depth: usize,
        label_name: Option<String>,
    ) {
        self.push_break_control(BreakControlKind::Loop, label_name, entry_depth, 0);
    }

    /// QuickJS restores the outer scope level on the for-in break entry while
    /// retaining its one hidden enumeration object across local continue.
    pub(in crate::engine::compiler) fn push_for_in_control(
        &mut self,
        entry_depth: usize,
        label_name: Option<String>,
        outer_scope: ScopeId,
    ) -> Result<(), Error> {
        let record_depth = entry_depth
            .checked_add(1)
            .ok_or_else(|| Error::new(ErrorKind::JsInternal, "stack overflow"))?;
        self.current_ir_mut()
            .context
            .break_controls
            .push(BreakControlContext {
                kind: BreakControlKind::ForIn,
                label_name,
                scope: outer_scope,
                entry_depth: record_depth,
                drop_count: 1,
                break_jumps: Vec::new(),
                continue_jumps: Vec::new(),
                finally_gosubs: Vec::new(),
            });
        Ok(())
    }

    /// QuickJS changes a for-of/for-await `BlockEnv`'s scope level back to the
    /// level outside the enumeration scope. Thus a same-loop break/continue
    /// closes the current lexical head cell while retaining the three-slot
    /// iterator record for the loop's shared next/close tail.
    pub(in crate::engine::compiler) fn push_for_of_control(
        &mut self,
        entry_depth: usize,
        label_name: Option<String>,
        outer_scope: ScopeId,
    ) -> Result<(), Error> {
        let record_depth = entry_depth
            .checked_add(3)
            .ok_or_else(|| Error::new(ErrorKind::JsInternal, "stack overflow"))?;
        self.current_ir_mut()
            .context
            .break_controls
            .push(BreakControlContext {
                kind: BreakControlKind::ForOf,
                label_name,
                scope: outer_scope,
                entry_depth: record_depth,
                drop_count: 3,
                break_jumps: Vec::new(),
                continue_jumps: Vec::new(),
                finally_gosubs: Vec::new(),
            });
        Ok(())
    }

    pub(in crate::engine::compiler) fn push_destructuring_iterator_control(
        &mut self,
    ) -> Result<(), Error> {
        let record_depth = self.current_ir().context.stack_depth;
        if record_depth < 3 {
            return Err(Error::internal(
                "destructuring iterator record is below the stack base",
            ));
        }
        self.push_break_control(
            BreakControlKind::DestructuringIterator,
            None,
            record_depth,
            3,
        );
        Ok(())
    }

    pub(in crate::engine::compiler) fn push_for_of_assignment_fragment_control(
        &mut self,
        entry_depth: usize,
    ) -> Result<(), Error> {
        let record_depth = entry_depth
            .checked_add(3)
            .ok_or_else(|| Error::new(ErrorKind::JsInternal, "stack overflow"))?;
        self.require_stack_depth(
            record_depth
                .checked_add(1)
                .ok_or_else(|| Error::new(ErrorKind::JsInternal, "stack overflow"))?,
            "for-of assignment fragment entry",
        )?;
        self.push_break_control(
            BreakControlKind::ForOfAssignmentFragment,
            None,
            record_depth,
            3,
        );
        Ok(())
    }

    pub(in crate::engine::compiler) fn pop_for_of_assignment_fragment_control(
        &mut self,
        entry_depth: usize,
    ) -> Result<(), Error> {
        let record_depth = entry_depth
            .checked_add(3)
            .ok_or_else(|| Error::new(ErrorKind::JsInternal, "stack overflow"))?;
        let control = self.pop_break_control()?;
        if control.kind != BreakControlKind::ForOfAssignmentFragment
            || control.label_name.is_some()
            || control.entry_depth != record_depth
            || control.drop_count != 3
            || !control.break_jumps.is_empty()
            || !control.continue_jumps.is_empty()
            || !control.finally_gosubs.is_empty()
        {
            return Err(Error::internal(
                "for-of assignment fragment control stack is unbalanced",
            ));
        }
        Ok(())
    }

    pub(in crate::engine::compiler) fn pop_destructuring_iterator_control(
        &mut self,
    ) -> Result<(), Error> {
        let control = self.pop_break_control()?;
        if control.kind != BreakControlKind::DestructuringIterator
            || control.label_name.is_some()
            || control.entry_depth
                != self
                    .current_ir()
                    .context
                    .stack_depth
                    .checked_add(3)
                    .ok_or_else(|| Error::new(ErrorKind::JsInternal, "stack overflow"))?
            || control.drop_count != 3
            || !control.break_jumps.is_empty()
            || !control.continue_jumps.is_empty()
            || !control.finally_gosubs.is_empty()
        {
            return Err(Error::internal(
                "destructuring iterator control stack is unbalanced",
            ));
        }
        Ok(())
    }

    pub(in crate::engine::compiler) fn push_break_control(
        &mut self,
        kind: BreakControlKind,
        label_name: Option<String>,
        entry_depth: usize,
        drop_count: usize,
    ) {
        let scope = self.current_ir().context.current_scope;
        self.current_ir_mut()
            .context
            .break_controls
            .push(BreakControlContext {
                kind,
                label_name,
                scope,
                entry_depth,
                drop_count,
                break_jumps: Vec::new(),
                continue_jumps: Vec::new(),
                finally_gosubs: Vec::new(),
            });
    }

    pub(in crate::engine::compiler) fn pop_break_control(
        &mut self,
    ) -> Result<BreakControlContext, Error> {
        self.current_ir_mut()
            .context
            .break_controls
            .pop()
            .ok_or_else(|| Error::internal("break control stack underflow"))
    }

    pub(in crate::engine::compiler) fn require_stack_depth(
        &self,
        expected: usize,
        construct: &str,
    ) -> Result<(), Error> {
        if self.current_ir().context.stack_depth == expected {
            Ok(())
        } else {
            Err(Error::internal(format!(
                "{construct} changed the enclosing stack depth"
            )))
        }
    }

    pub(in crate::engine::compiler) fn finish_control_statement(&mut self) {
        self.current_ir_mut().context.last_member_reference = None;
        self.current_ir_mut().context.last_identifier_reference = None;
        self.current_ir_mut().context.last_optional_chain = None;
        self.anonymous_function_definition = None;
    }

    /// Emit QuickJS's shared return path for an already-evaluated value at
    /// TOS. Generator resumption uses this same path when `.return(value)` is
    /// injected at a `yield`, so iterator/finally unwinding remains identical
    /// to an authored ReturnStatement.
    pub(in crate::engine::compiler) fn emit_return_completion(
        &mut self,
        return_span: Span,
        await_async_generator_value: bool,
    ) -> Result<(), Error> {
        if await_async_generator_value
            && self.current_ir().execution_kind == BytecodeFunctionKind::AsyncGenerator
        {
            // Pinned QuickJS performs this await before any iterator-close or
            // finally work so a rejected return value wins with the same
            // observable ordering as `emit_return`.
            self.emit_instruction_at(Instruction::Await, source_offset(return_span)?)?;
        }
        let async_iterator_return =
            if self.current_ir().execution_kind == BytecodeFunctionKind::AsyncGenerator
                && self
                    .current_ir()
                    .context
                    .break_controls
                    .iter()
                    .any(|control| {
                        matches!(
                            control.kind,
                            BreakControlKind::DestructuringIterator | BreakControlKind::ForOf
                        )
                    })
            {
                Some(self.add_constant(IrConstant::Primitive(Value::String(
                    JsString::from_static("return"),
                )))?)
            } else {
                None
            };
        // QuickJS walks BlockEnv entries from inner to outer and interleaves
        // iterator closing with finally execution. Keeping that order is
        // observable when either an iterator `return` method or a finally body
        // throws, and is also required for the VM's nested unwind regions.
        let unwind_controls = self
            .current_ir()
            .context
            .break_controls
            .iter()
            .enumerate()
            .rev()
            .filter_map(|(index, control)| {
                matches!(
                    control.kind,
                    BreakControlKind::DestructuringIterator
                        | BreakControlKind::ForOfAssignmentFragment
                        | BreakControlKind::ForOf
                        | BreakControlKind::TryFinally
                )
                .then_some((index, control.kind, control.entry_depth))
            })
            .collect::<Vec<_>>();
        for (control_index, kind, handler_depth) in unwind_controls {
            match kind {
                BreakControlKind::DestructuringIterator | BreakControlKind::ForOf => {
                    if handler_depth < 3 || handler_depth >= self.current_ir().context.stack_depth {
                        return Err(Error::internal(
                            "return unwind targeted an invalid iterator record",
                        ));
                    }
                    if let Some(return_name) = async_iterator_return {
                        // Pinned QuickJS hand-lowers AsyncGenerator return
                        // cleanup instead of using OP_iterator_close:
                        //
                        //   iter next ... completion
                        //     -> completion iter
                        //     -> completion iter return
                        //
                        // A nullish return method is ignored. Otherwise it is
                        // called without arguments, its immediate result is
                        // checked as an Object *before* Await, and the settled
                        // value is discarded without a second object check.
                        self.emit_instruction(Instruction::IteratorDetachPreserve)?;
                        self.current_ir_mut().context.stack_depth = handler_depth - 1;
                        self.emit_instruction(Instruction::GetField2(return_name))?;
                        self.emit_instruction(Instruction::Dup)?;
                        self.emit_instruction(Instruction::IsUndefinedOrNull)?;
                        let missing_return =
                            self.emit_instruction(Instruction::IfTrue(u32::MAX))?;
                        self.emit_instruction(Instruction::CallMethod(0))?;
                        self.emit_instruction(Instruction::IteratorCheckObject)?;
                        self.emit_instruction(Instruction::Await)?;
                        let close_done = self.emit_instruction(Instruction::Goto(u32::MAX))?;

                        let missing_target = self.current_ir().ops.len();
                        self.current_ir_mut().context.stack_depth = handler_depth;
                        self.patch_jump(missing_return, missing_target)?;
                        // Drop the nullish method; the common tail then drops
                        // the retained iterator or settled close result.
                        self.emit_instruction(Instruction::Drop)?;
                        let close_done_target = self.current_ir().ops.len();
                        self.patch_jump(close_done, close_done_target)?;
                        self.require_stack_depth(
                            handler_depth - 1,
                            "async-generator iterator-return branch",
                        )?;
                        self.emit_instruction(Instruction::Drop)?;
                        self.current_ir_mut().context.stack_depth = handler_depth - 2;
                    } else {
                        self.emit_instruction(Instruction::IteratorClosePreserve)?;
                        // The generic instruction effect is value preserving,
                        // but this typed form also truncates the complete
                        // iterator record and any intermediate finally
                        // operands.
                        self.current_ir_mut().context.stack_depth = handler_depth - 2;
                    }
                }
                BreakControlKind::ForOfAssignmentFragment => {
                    if handler_depth < 3 || handler_depth >= self.current_ir().context.stack_depth {
                        return Err(Error::internal(
                            "return unwind targeted an invalid for-of assignment record",
                        ));
                    }
                    self.emit_instruction(Instruction::IteratorDropPreserve)?;
                    // As in pinned QuickJS, leaving the precompiled head
                    // assignment abandons the outer record without invoking
                    // IteratorClose.
                    self.current_ir_mut().context.stack_depth = handler_depth - 2;
                }
                BreakControlKind::TryFinally => {
                    // Preserve the return value while removing everything
                    // through the nearest catch marker, then call the
                    // associated finally body.
                    self.emit_instruction(Instruction::NipCatch)?;
                    if handler_depth > self.current_ir().context.stack_depth {
                        return Err(Error::internal(
                            "return unwind targeted a deeper catch marker",
                        ));
                    }
                    self.current_ir_mut().context.stack_depth = handler_depth;
                    self.require_stack_depth(handler_depth, "return catch cleanup")?;
                    let gosub = self.emit_instruction(Instruction::Gosub(u32::MAX))?;
                    self.current_ir_mut().context.break_controls[control_index]
                        .finally_gosubs
                        .push(gosub);
                }
                _ => unreachable!("return unwind list contains an ordinary control"),
            }
        }
        let instruction = if self.current_ir().derived_class_constructor {
            Instruction::ReturnDerived(
                self.current_ir()
                    .this_local
                    .ok_or_else(|| Error::internal("derived constructor has no this binding"))?,
            )
        } else {
            Instruction::Return
        };
        self.emit_instruction_at(instruction, source_offset(return_span)?)?;
        Ok(())
    }
}
