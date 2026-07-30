//! QuickJS-shaped optional-chain parsing and control-flow rewrites.
//!
//! The VM needs no optional-chain opcode: the compiler lowers each nullish
//! edge through Dup/IsUndefinedOrNull/IfFalse and sends every short circuit to
//! one chain-end label. Parser-only metadata preserves the few Reference
//! rewrites which can still happen outside a parenthesized chain.

use super::*;

/// One nullish edge in a parser-owned optional chain.
///
/// A zero-effect slot beside the fallback `undefined` lets a later grouped
/// method call add its receiver without inserting IR or shifting jump indices.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct OptionalChainShortCircuit {
    fallback: usize,
    receiver_padding: usize,
    jump: usize,
}

#[derive(Debug, Default)]
pub(super) struct PendingOptionalChain {
    short_circuits: Vec<OptionalChainShortCircuit>,
}

/// Marker retained across parentheses until an outer operation composes the
/// chain value or consumes its terminal member Reference.
#[derive(Clone, Debug)]
pub(super) struct FinalizedOptionalChain {
    short_circuits: Vec<OptionalChainShortCircuit>,
    pub(super) terminal_member_get: Option<usize>,
}

impl<'source> Parser<'source> {
    /// Emit QuickJS's shared optional-chain test while retaining a spare Nop
    /// beside the fallback value.
    pub(super) fn emit_optional_chain_test(
        &mut self,
        chain: &mut PendingOptionalChain,
        drop_count: usize,
    ) -> Result<(), Error> {
        let continuation_depth = self.current_ir().stack_depth;
        if drop_count == 0 || continuation_depth < drop_count {
            return Err(Error::internal(
                "optional chain test has an invalid Reference depth",
            ));
        }
        self.emit_instruction(Instruction::Dup)?;
        self.emit_instruction(Instruction::IsUndefinedOrNull)?;
        let continue_jump = self.emit_instruction(Instruction::IfFalse(u32::MAX))?;
        for _ in 0..drop_count {
            self.emit_instruction(Instruction::Drop)?;
        }
        let fallback = self.emit_instruction(Instruction::Undefined)?;
        let receiver_padding = self.emit_instruction(Instruction::Nop)?;
        let short_circuit = self.emit_instruction(Instruction::Goto(u32::MAX))?;
        self.patch_jump(continue_jump, self.current_ir().ops.len())?;
        self.current_ir_mut().stack_depth = continuation_depth;
        chain.short_circuits.push(OptionalChainShortCircuit {
            fallback,
            receiver_padding,
            jump: short_circuit,
        });
        Ok(())
    }

    /// Parse the member suffix after an already-consumed `?.`. Optional calls
    /// stay in the ordinary call path because their test needs the promoted
    /// callee Reference depth.
    pub(super) fn parse_optional_member_suffix(&mut self, member_span: Span) -> Result<(), Error> {
        if self.is_punctuator(Punctuator::LeftBracket) {
            self.advance_expression_start()?;
            self.parse_expression()?;
            self.expect_punctuator(Punctuator::RightBracket)?;
            let operation =
                self.emit_instruction_at(Instruction::GetArrayEl, source_offset(member_span)?)?;
            self.current_ir_mut().last_member_reference = Some(operation);
            self.anonymous_function_definition = None;
            return Ok(());
        }

        let token = self.current().clone();
        let name = match token.kind {
            TokenKind::PrivateIdentifier(identifier) => {
                let name = private_reference::private_binding_name(&identifier.value);
                self.advance()?;
                let operation =
                    self.emit_private_field_get(name, token.span, source_offset(member_span)?)?;
                self.current_ir_mut().last_member_reference = Some(operation);
                self.anonymous_function_definition = None;
                return Ok(());
            }
            TokenKind::Identifier(identifier) => identifier.value,
            TokenKind::Keyword(keyword) => keyword.as_str().to_owned(),
            _ => return Err(self.syntax_here("expecting field name")),
        };
        self.advance()?;
        let key = self.add_constant(IrConstant::Primitive(Value::String(
            JsString::try_from_utf8(&name)?,
        )))?;
        let operation =
            self.emit_instruction_at(Instruction::GetField(key), source_offset(member_span)?)?;
        self.current_ir_mut().last_member_reference = Some(operation);
        self.anonymous_function_definition = None;
        Ok(())
    }

    pub(super) fn finish_optional_chain(
        &mut self,
        chain: PendingOptionalChain,
    ) -> Result<(), Error> {
        if chain.short_circuits.is_empty() {
            return Err(Error::internal(
                "optional chain finished without a nullish edge",
            ));
        }
        let (terminal_member_get, terminal_private_get) = {
            let function = self.current_ir();
            let terminal_member_get =
                if function.last_member_reference == function.ops.len().checked_sub(1) {
                    function.last_member_reference
                } else {
                    None
                };
            let terminal_private_get = terminal_member_get.is_some()
                && matches!(
                    function.ops.last(),
                    Some(SpannedIrOp {
                        op: IrOp::PrivateField {
                            access: PrivateFieldAccess::Get,
                            ..
                        },
                        ..
                    })
                );
            (terminal_member_get, terminal_private_get)
        };
        let end = self.current_ir().ops.len();
        for short_circuit in &chain.short_circuits {
            self.patch_jump(short_circuit.jump, end)?;
        }
        let function = self.current_ir_mut();
        // QuickJS recognizes only public field/computed getters in its chain
        // close marker. Thus `(obj?.#m)()` loses the receiver, while
        // `obj?.#m()` was already promoted inside the active chain.
        if terminal_private_get {
            function.last_member_reference = None;
        }
        function.last_optional_chain = Some(FinalizedOptionalChain {
            short_circuits: chain.short_circuits,
            terminal_member_get: (!terminal_private_get)
                .then_some(terminal_member_get)
                .flatten(),
        });
        Ok(())
    }

    /// Make every nullish edge bypass a terminal public Delete and return
    /// true. The non-nullish getter has already been restored to raw Reference
    /// operands by `take_tail_member_reference`.
    pub(super) fn rewrite_optional_chain_delete_fallback(
        &mut self,
        chain: FinalizedOptionalChain,
    ) -> Result<(), Error> {
        let end = self.current_ir().ops.len();
        for short_circuit in chain.short_circuits {
            let Some(SpannedIrOp {
                op: IrOp::Bytecode(fallback),
                ..
            }) = self.current_ir_mut().ops.get_mut(short_circuit.fallback)
            else {
                return Err(Error::internal(
                    "optional chain delete fallback disappeared",
                ));
            };
            if !matches!(fallback, Instruction::Undefined) {
                return Err(Error::internal(
                    "optional chain delete fallback was already rewritten",
                ));
            }
            *fallback = Instruction::PushTrue;
            self.patch_jump(short_circuit.jump, end)?;
        }
        Ok(())
    }
}

/// A call or tagged template outside `(optional?.chain)` preserves a public
/// terminal property Reference. Pad each short branch to the two-value method
/// ABI when that getter is promoted.
pub(super) fn pad_grouped_method_receiver(
    function: &mut FunctionIr,
    terminal_get: Option<usize>,
) -> Result<(), Error> {
    if function
        .last_optional_chain
        .as_ref()
        .is_none_or(|chain| chain.terminal_member_get != terminal_get)
    {
        return Ok(());
    }
    let chain = function
        .last_optional_chain
        .take()
        .ok_or_else(|| Error::internal("optional chain marker disappeared"))?;
    for short_circuit in chain.short_circuits {
        let Some(SpannedIrOp {
            op: IrOp::Bytecode(padding),
            ..
        }) = function.ops.get_mut(short_circuit.receiver_padding)
        else {
            return Err(Error::internal(
                "optional chain receiver padding disappeared",
            ));
        };
        if !matches!(padding, Instruction::Nop) {
            return Err(Error::internal(
                "optional chain receiver padding was already consumed",
            ));
        }
        *padding = Instruction::Undefined;
    }
    Ok(())
}
