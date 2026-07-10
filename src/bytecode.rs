use crate::Error;
use crate::value::Value;
use std::collections::VecDeque;

/// QuickJS `JS_STACK_SIZE_MAX` for one bytecode function.
pub const MAX_STACK_SIZE: u16 = 65_534;

/// Stack-machine operations deliberately use the names and stack behavior of
/// their `QuickJS` counterparts. This typed form is the current compiler IR and
/// verified execution format; a future compact encoder must share this opcode
/// metadata instead of defining a second instruction contract.
#[derive(Clone, Debug)]
pub enum Instruction {
    Nop,
    PushI32(i32),
    PushConst(u32),
    FClosure(u32),
    /// QuickJS `OP_set_name`: conditionally define the name of the object at
    /// the top of the stack from a string constant, without consuming it.
    SetName(u32),
    /// QuickJS `OP_throw_error atom JS_THROW_VAR_RO` for a strict immutable
    /// function-expression name. Consumes the attempted value and terminates.
    ThrowReadOnly(u32),
    Undefined,
    Null,
    PushFalse,
    PushTrue,
    PushThis,
    PushNewTarget,
    GetLocal(u16),
    PutLocal(u16),
    SetLocal(u16),
    GetArg(u16),
    PutArg(u16),
    SetArg(u16),
    GetVarRef(u16),
    PutVarRef(u16),
    SetVarRef(u16),
    /// QuickJS `OP_get_var`: read a global-environment VarRef closure slot.
    GetVar(u16),
    /// QuickJS `OP_get_var_undef`: as `GetVar`, but suppress a genuinely
    /// missing global binding for a direct `typeof IdentifierReference`.
    GetVarUndef(u16),
    /// QuickJS `OP_put_var`: assign and consume a global binding value.
    PutVar(u16),
    /// QuickJS `OP_put_var_init`: initialize a lexical global binding.
    PutVarInit(u16),
    /// QuickJS `OP_get_field`: replace an object-like base with the value of
    /// one constant string-keyed property.
    GetField(u32),
    /// QuickJS `OP_get_field2`: keep the base below the fetched value so a
    /// following `CallMethod` observes the original reference receiver.
    GetField2(u32),
    /// QuickJS `OP_get_array_el`: `base key -> value`, including observable
    /// `ToPropertyKey` conversion in the runtime host.
    GetArrayEl,
    /// QuickJS `OP_get_array_el2`: `base key -> base value` for method calls.
    GetArrayEl2,
    /// QuickJS `OP_get_array_el3`: `base raw-key -> base converted-key value`
    /// for compound/logical assignment without repeated key conversion.
    GetArrayEl3,
    /// QuickJS `OP_to_propkey`: observable `ToPropertyKey` while retaining a
    /// canonical Int/String/Symbol value on the VM stack.
    ToPropKey,
    /// QuickJS `OP_insert2`: `base value -> value base value`.
    Insert2,
    /// QuickJS `OP_insert3`: `base key value -> value base key value`.
    Insert3,
    /// QuickJS `OP_put_field`: assign one constant string-keyed property.
    PutField(u32),
    /// QuickJS `OP_put_array_el`: assign a computed property, converting the
    /// still-raw key only after the right-hand side has been evaluated.
    PutArrayEl,
    /// QuickJS `OP_delete`: `base key -> bool` with strictness supplied by the
    /// active call frame.
    Delete,
    Drop,
    /// QuickJS `OP_nip`: discard the value immediately below the stack top,
    /// preserving the top value (`a b -> b`).
    Nip,
    Dup,
    Neg,
    Plus,
    BitNot,
    Not,
    TypeOf,
    IsUndefinedOrNull,
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Shl,
    Sar,
    Shr,
    BitAnd,
    BitXor,
    BitOr,
    Eq,
    StrictEq,
    Neq,
    StrictNeq,
    Lt,
    Lte,
    Gt,
    Gte,
    IfFalse(u32),
    IfTrue(u32),
    Goto(u32),
    Call(u16),
    CallMethod(u16),
    /// QuickJS `OP_call_constructor`: `func new.target args -> result`.
    Construct(u16),
    Return,
    Throw,
}

impl Instruction {
    #[must_use]
    pub const fn stack_effect(&self) -> (usize, usize) {
        match self {
            Self::Nop | Self::Goto(_) => (0, 0),
            Self::PushI32(_)
            | Self::PushConst(_)
            | Self::FClosure(_)
            | Self::Undefined
            | Self::Null
            | Self::PushFalse
            | Self::PushTrue
            | Self::PushThis
            | Self::PushNewTarget
            | Self::GetLocal(_)
            | Self::GetArg(_)
            | Self::GetVarRef(_)
            | Self::GetVar(_)
            | Self::GetVarUndef(_) => (0, 1),
            Self::SetName(_) => (1, 1),
            Self::GetField(_) => (1, 1),
            Self::GetField2(_) => (1, 2),
            Self::GetArrayEl => (2, 1),
            Self::GetArrayEl2 => (2, 2),
            Self::GetArrayEl3 => (2, 3),
            Self::ToPropKey => (1, 1),
            Self::Insert2 => (2, 3),
            Self::Insert3 => (3, 4),
            Self::PutField(_) => (2, 0),
            Self::PutArrayEl => (3, 0),
            Self::Delete => (2, 1),
            Self::Call(argument_count) => (*argument_count as usize + 1, 1),
            Self::CallMethod(argument_count) => (*argument_count as usize + 2, 1),
            Self::Construct(argument_count) => (*argument_count as usize + 2, 1),
            Self::Drop
            | Self::PutLocal(_)
            | Self::PutArg(_)
            | Self::PutVarRef(_)
            | Self::PutVar(_)
            | Self::PutVarInit(_)
            | Self::ThrowReadOnly(_)
            | Self::IfFalse(_)
            | Self::IfTrue(_)
            | Self::Return
            | Self::Throw => (1, 0),
            Self::Nip => (2, 1),
            Self::SetLocal(_) | Self::SetArg(_) | Self::SetVarRef(_) => (1, 1),
            Self::Dup => (1, 2),
            Self::Neg
            | Self::Plus
            | Self::BitNot
            | Self::Not
            | Self::TypeOf
            | Self::IsUndefinedOrNull => (1, 1),
            Self::Add
            | Self::Sub
            | Self::Mul
            | Self::Div
            | Self::Mod
            | Self::Shl
            | Self::Sar
            | Self::Shr
            | Self::BitAnd
            | Self::BitXor
            | Self::BitOr
            | Self::Eq
            | Self::StrictEq
            | Self::Neq
            | Self::StrictNeq
            | Self::Lt
            | Self::Lte
            | Self::Gt
            | Self::Gte => (2, 1),
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct BytecodeFunction {
    pub name: Option<String>,
    pub code: Vec<Instruction>,
    pub constants: Vec<Value>,
    pub max_stack: u16,
}

impl BytecodeFunction {
    #[must_use]
    pub fn constant(&self, index: u32) -> Option<&Value> {
        usize::try_from(index)
            .ok()
            .and_then(|index| self.constants.get(index))
    }

    /// Validate control flow, operands, and stack depth before execution.
    ///
    /// # Errors
    /// Returns an internal error when bytecode is malformed. Source compilation
    /// must never produce such an error; decoders must reject it before use.
    pub fn verify(&self) -> Result<VerifiedBytecode, Error> {
        for instruction in &self.code {
            if let Instruction::SetName(index)
            | Instruction::ThrowReadOnly(index)
            | Instruction::GetField(index)
            | Instruction::GetField2(index)
            | Instruction::PutField(index) = instruction
                && !matches!(self.constant(*index), Some(Value::String(_)))
            {
                return Err(Error::internal(
                    "string-key opcode referenced a non-string constant",
                ));
            }
        }
        verify_parts(&self.code, self.constants.len(), self.max_stack)
    }
}

/// Verify immutable bytecode parts before they enter the runtime heap.
///
/// Constant kinds are runtime-owned after linking, but control-flow validation
/// only needs the pool length. Keeping this verifier representation-neutral
/// lets publication validate child-function constants without manufacturing
/// temporary public [`Value`]s.
pub(crate) fn verify_parts(
    code: &[Instruction],
    constant_count: usize,
    declared_max_stack: u16,
) -> Result<VerifiedBytecode, Error> {
    if declared_max_stack > MAX_STACK_SIZE {
        return Err(Error::internal(
            "declared bytecode stack exceeds QuickJS JS_STACK_SIZE_MAX",
        ));
    }
    if code.is_empty() {
        return Err(Error::internal("bytecode function has no instructions"));
    }

    // Publication is a trust boundary. Validate representation operands for
    // every instruction, including dead code, before the reachability walk.
    // Later kind-specific verification may then index the constant pool
    // without letting malformed unreachable bytecode panic the runtime.
    for instruction in code {
        match instruction {
            Instruction::PushConst(index)
            | Instruction::FClosure(index)
            | Instruction::SetName(index)
            | Instruction::ThrowReadOnly(index)
            | Instruction::GetField(index)
            | Instruction::GetField2(index)
            | Instruction::PutField(index) => {
                let is_valid = usize::try_from(*index)
                    .ok()
                    .is_some_and(|index| index < constant_count);
                if !is_valid {
                    return Err(Error::internal("constant index is out of bounds"));
                }
            }
            Instruction::Goto(target)
            | Instruction::IfFalse(target)
            | Instruction::IfTrue(target) => {
                validate_target(*target, code.len())?;
            }
            _ => {}
        }
    }

    let mut depths = vec![None; code.len()];
    let mut worklist = VecDeque::from([(0_usize, 0_usize)]);
    let mut maximum = 0_usize;
    let mut has_termination = false;

    while let Some((pc, depth)) = worklist.pop_front() {
        let slot = depths
            .get_mut(pc)
            .ok_or_else(|| Error::internal("control flow target is out of bounds"))?;
        if let Some(previous) = *slot {
            if previous != depth {
                return Err(Error::internal(
                    "control flow joins with inconsistent stack depth",
                ));
            }
            continue;
        }
        *slot = Some(depth);

        let instruction = &code[pc];
        let (popped, pushed) = instruction.stack_effect();
        let next_depth = depth
            .checked_sub(popped)
            .ok_or_else(|| Error::internal("bytecode stack underflow"))?
            .checked_add(pushed)
            .ok_or_else(|| Error::internal("bytecode stack depth overflow"))?;
        maximum = maximum.max(next_depth);

        match instruction {
            Instruction::Return | Instruction::Throw | Instruction::ThrowReadOnly(_) => {
                if next_depth != 0 {
                    return Err(Error::internal(
                        "function completion leaves temporary values on the bytecode stack",
                    ));
                }
                has_termination = true;
            }
            Instruction::Goto(target) => {
                enqueue_target(&mut worklist, *target, next_depth, code.len())?;
            }
            Instruction::IfFalse(target) | Instruction::IfTrue(target) => {
                enqueue_target(&mut worklist, *target, next_depth, code.len())?;
                enqueue_fallthrough(&mut worklist, pc, next_depth, code.len())?;
            }
            _ => enqueue_fallthrough(&mut worklist, pc, next_depth, code.len())?,
        }
    }

    if !has_termination {
        return Err(Error::internal("bytecode has no reachable return or throw"));
    }
    let maximum =
        u16::try_from(maximum).map_err(|_| Error::internal("bytecode stack exceeds u16::MAX"))?;
    if maximum > declared_max_stack {
        return Err(Error::internal(
            "declared maximum stack is smaller than required",
        ));
    }
    Ok(VerifiedBytecode { max_stack: maximum })
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct VerifiedBytecode {
    pub max_stack: u16,
}

fn enqueue_target(
    worklist: &mut VecDeque<(usize, usize)>,
    target: u32,
    depth: usize,
    code_len: usize,
) -> Result<(), Error> {
    let target = validate_target(target, code_len)?;
    worklist.push_back((target, depth));
    Ok(())
}

fn validate_target(target: u32, code_len: usize) -> Result<usize, Error> {
    let target = usize::try_from(target).map_err(|_| Error::internal("jump target overflow"))?;
    if target >= code_len {
        return Err(Error::internal("jump target is out of bounds"));
    }
    Ok(target)
}

fn enqueue_fallthrough(
    worklist: &mut VecDeque<(usize, usize)>,
    pc: usize,
    depth: usize,
    code_len: usize,
) -> Result<(), Error> {
    let next = pc
        .checked_add(1)
        .ok_or_else(|| Error::internal("program counter overflow"))?;
    if next >= code_len {
        return Err(Error::internal("bytecode ended without return"));
    }
    worklist.push_back((next, depth));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{BytecodeFunction, Instruction};
    use crate::Value;

    #[test]
    fn verifier_computes_reachable_stack_depth() {
        let function = BytecodeFunction {
            name: None,
            code: vec![
                Instruction::PushTrue,
                Instruction::IfFalse(4),
                Instruction::PushI32(1),
                Instruction::Goto(5),
                Instruction::PushI32(2),
                Instruction::Return,
            ],
            constants: vec![],
            max_stack: 1,
        };
        assert_eq!(function.verify().unwrap().max_stack, 1);
    }

    #[test]
    fn verifier_rejects_bad_constants_and_stack_joins() {
        let bad_constant = BytecodeFunction {
            name: None,
            code: vec![Instruction::PushConst(0), Instruction::Return],
            constants: vec![],
            max_stack: 1,
        };
        assert!(bad_constant.verify().is_err());

        let bad_join = BytecodeFunction {
            name: None,
            code: vec![
                Instruction::PushTrue,
                Instruction::IfFalse(4),
                Instruction::PushI32(1),
                Instruction::Goto(5),
                Instruction::Goto(5),
                Instruction::PushI32(2),
                Instruction::Return,
            ],
            constants: vec![Value::Undefined],
            max_stack: 2,
        };
        assert!(bad_join.verify().is_err());

        let excessive_declared_stack = BytecodeFunction {
            name: None,
            code: vec![Instruction::Undefined, Instruction::Return],
            constants: vec![],
            max_stack: u16::MAX,
        };
        assert!(excessive_declared_stack.verify().is_err());

        let valid_nip = BytecodeFunction {
            name: None,
            code: vec![
                Instruction::PushI32(1),
                Instruction::PushI32(2),
                Instruction::Nip,
                Instruction::Return,
            ],
            constants: vec![],
            max_stack: 2,
        };
        assert_eq!(valid_nip.verify().unwrap().max_stack, 2);

        let nip_underflow = BytecodeFunction {
            name: None,
            code: vec![
                Instruction::PushI32(1),
                Instruction::Nip,
                Instruction::Return,
            ],
            constants: vec![],
            max_stack: 1,
        };
        assert!(nip_underflow.verify().is_err());
    }

    #[test]
    fn verifier_rejects_malformed_operands_even_in_unreachable_code() {
        let bad_constant = BytecodeFunction {
            name: None,
            code: vec![
                Instruction::Undefined,
                Instruction::Return,
                Instruction::PushConst(99),
            ],
            constants: vec![],
            max_stack: 1,
        };
        assert!(bad_constant.verify().is_err());

        let bad_set_name = BytecodeFunction {
            name: None,
            code: vec![
                Instruction::Undefined,
                Instruction::Return,
                Instruction::SetName(99),
            ],
            constants: vec![],
            max_stack: 1,
        };
        assert!(bad_set_name.verify().is_err());

        let non_string_set_name = BytecodeFunction {
            name: None,
            code: vec![
                Instruction::Undefined,
                Instruction::SetName(0),
                Instruction::Return,
            ],
            constants: vec![Value::Int(1)],
            max_stack: 1,
        };
        assert!(non_string_set_name.verify().is_err());

        let non_string_read_only_name = BytecodeFunction {
            name: None,
            code: vec![Instruction::PushI32(1), Instruction::ThrowReadOnly(0)],
            constants: vec![Value::Int(1)],
            max_stack: 1,
        };
        assert!(non_string_read_only_name.verify().is_err());

        let non_string_field = BytecodeFunction {
            name: None,
            code: vec![
                Instruction::Undefined,
                Instruction::GetField(0),
                Instruction::Return,
            ],
            constants: vec![Value::Int(1)],
            max_stack: 1,
        };
        assert!(non_string_field.verify().is_err());

        let non_string_keep_field = BytecodeFunction {
            name: None,
            code: vec![
                Instruction::Undefined,
                Instruction::GetField2(0),
                Instruction::Drop,
                Instruction::Return,
            ],
            constants: vec![Value::Int(1)],
            max_stack: 2,
        };
        assert!(non_string_keep_field.verify().is_err());

        let non_string_put_field = BytecodeFunction {
            name: None,
            code: vec![
                Instruction::Undefined,
                Instruction::Undefined,
                Instruction::PutField(0),
                Instruction::Undefined,
                Instruction::Return,
            ],
            constants: vec![Value::Int(1)],
            max_stack: 2,
        };
        assert!(non_string_put_field.verify().is_err());

        let bad_jump = BytecodeFunction {
            name: None,
            code: vec![
                Instruction::Undefined,
                Instruction::Return,
                Instruction::Goto(99),
            ],
            constants: vec![],
            max_stack: 1,
        };
        assert!(bad_jump.verify().is_err());
    }
}
