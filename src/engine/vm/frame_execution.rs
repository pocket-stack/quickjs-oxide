use super::*;

impl VmActivation {
    #[cfg(test)]
    pub(in crate::engine::vm) fn new(max_stack: usize) -> Self {
        Self {
            stack: Vec::with_capacity(max_stack),
            regions: Vec::new(),
            pc: 0,
            caller_realm: None,
            callee_realm: None,
            current_function: None,
            this_value: Value::Undefined,
            normalized_this: None,
            new_target: Value::Undefined,
            strict: true,
            callee_global: None,
        }
    }

    pub(in crate::engine::vm) fn new_in_realm(
        metadata: FunctionMetadata,
        caller_realm: ContextId,
        callee_realm: ContextId,
        current_function: ObjectRef,
        this_value: Value,
        new_target: Value,
        callee_global: ObjectRef,
    ) -> Self {
        Self {
            stack: Vec::with_capacity(usize::from(metadata.max_stack)),
            regions: Vec::new(),
            pc: 0,
            caller_realm: Some(caller_realm),
            callee_realm: Some(callee_realm),
            current_function: Some(current_function),
            this_value,
            normalized_this: None,
            new_target,
            strict: metadata.strict,
            callee_global: Some(callee_global),
        }
    }

    /// Split an inactive VM activation into fields the runtime can map to its
    /// raw generator-frame representation.
    pub(crate) fn into_parts(self) -> VmActivationParts {
        VmActivationParts {
            stack: self.stack,
            regions: self.regions,
            pc: self.pc,
            caller_realm: self.caller_realm,
            callee_realm: self.callee_realm,
            current_function: self.current_function,
            this_value: self.this_value,
            normalized_this: self.normalized_this,
            new_target: self.new_target,
            strict: self.strict,
            callee_global: self.callee_global,
        }
    }

    /// Rebuild an inactive activation after the runtime has rooted its raw
    /// generator-frame values for an individual resume operation.
    pub(crate) fn from_parts(parts: VmActivationParts) -> Self {
        Self {
            stack: parts.stack,
            regions: parts.regions,
            pc: parts.pc,
            caller_realm: parts.caller_realm,
            callee_realm: parts.callee_realm,
            current_function: parts.current_function,
            this_value: parts.this_value,
            normalized_this: parts.normalized_this,
            new_target: parts.new_target,
            strict: parts.strict,
            callee_global: parts.callee_global,
        }
    }

    pub(in crate::engine::vm) fn execute(
        mut self,
        code: &[Instruction],
        host: &mut impl VmHost,
    ) -> Result<Completion, Error> {
        loop {
            let raised = match self.execute_inner(code, host) {
                Ok(InterpreterExit::Complete(Completion::Return(value))) => {
                    return Ok(Completion::Return(value));
                }
                Ok(InterpreterExit::Complete(Completion::Throw(value))) => value,
                Ok(InterpreterExit::Suspend(_)) => {
                    return Err(Error::internal(
                        "VM suspension reached execute-to-completion entry",
                    ));
                }
                Err(error) if NativeErrorKind::from_javascript_error(error.kind()).is_some() => {
                    host.materialize_error(error)?
                }
                Err(error) => return Err(error),
            };
            if let Some(completion) = self.raise(raised, host, code.len())? {
                return Ok(completion);
            }
        }
    }

    pub(in crate::engine::vm) fn run(
        mut self,
        code: &[Instruction],
        host: &mut impl VmHost,
    ) -> Result<VmExit, Error> {
        loop {
            let raised = match self.execute_inner(code, host) {
                Ok(InterpreterExit::Complete(Completion::Return(value))) => {
                    return Ok(VmExit::Complete(Completion::Return(value)));
                }
                Ok(InterpreterExit::Complete(Completion::Throw(value))) => value,
                Ok(InterpreterExit::Suspend(kind)) => {
                    return VmSuspension::new(kind, self).map(VmExit::Suspend);
                }
                Err(error) if NativeErrorKind::from_javascript_error(error.kind()).is_some() => {
                    host.materialize_error(error)?
                }
                Err(error) => return Err(error),
            };
            if let Some(completion) = self.raise(raised, host, code.len())? {
                return Ok(VmExit::Complete(completion));
            }
        }
    }

    pub(in crate::engine::vm) fn execute_inner(
        &mut self,
        code: &[Instruction],
        host: &mut impl VmHost,
    ) -> Result<InterpreterExit, Error> {
        loop {
            let instruction = code
                .get(self.pc)
                .ok_or_else(|| Error::internal("bytecode ended without return"))?;
            host.update_active_bytecode_pc(BytecodePc::new(self.pc))?;
            self.pc = self
                .pc
                .checked_add(1)
                .ok_or_else(|| Error::internal("program counter overflow"))?;

            let suspension = match instruction {
                Instruction::InitialYield => Some(VmSuspendKind::Initial),
                Instruction::Yield => Some(VmSuspendKind::Yield),
                Instruction::YieldStar => Some(VmSuspendKind::YieldStar),
                Instruction::AsyncYieldStar => Some(VmSuspendKind::AsyncYieldStar),
                Instruction::Await => Some(VmSuspendKind::Await),
                _ => None,
            };
            if let Some(kind) = suspension {
                return Ok(InterpreterExit::Suspend(kind));
            }

            if matches!(
                instruction,
                Instruction::Arguments(_)
                    | Instruction::Rest(_)
                    | Instruction::VariableEnvironment
                    | Instruction::HasEvalVariable { .. }
                    | Instruction::GetEvalVariable { .. }
                    | Instruction::PutEvalVariable { .. }
                    | Instruction::DeleteEvalVariable { .. }
                    | Instruction::DefineEvalVariable { .. }
                    | Instruction::ToObject
                    | Instruction::HasDynamicBinding { .. }
                    | Instruction::GetDynamicBinding { .. }
                    | Instruction::PutDynamicBinding { .. }
                    | Instruction::DeleteDynamicBinding { .. }
                    | Instruction::DynamicEnvironmentObject(_)
                    | Instruction::GlobalReference(_)
                    | Instruction::GetRefValue(_)
                    | Instruction::GetRefValueUndef(_)
                    | Instruction::PutRefValue(_)
                    | Instruction::Object
                    | Instruction::RegExp(_)
                    | Instruction::SetNameComputed
                    | Instruction::DefineMethod { .. }
                    | Instruction::DefineMethodComputed { .. }
                    | Instruction::DefineClass { .. }
                    | Instruction::SetProto
                    | Instruction::CopyDataProperties
                    | Instruction::CopyDataPropertiesExcluded { .. }
                    | Instruction::IteratorStart
                    | Instruction::AsyncIteratorStart
                    | Instruction::IteratorNext
                    | Instruction::IteratorCall(_)
                    | Instruction::ForAwaitOfStart
                    | Instruction::ForAwaitOfNext
                    | Instruction::IteratorGetValueDone
                    | Instruction::ForInStart
                    | Instruction::ForInNext
            ) {
                if let Some(completion) = self.execute_cold_instruction(instruction, host)? {
                    return Ok(InterpreterExit::Complete(completion));
                }
                continue;
            }

            if matches!(
                instruction,
                Instruction::Import
                    | Instruction::Call(_)
                    | Instruction::TailCall(_)
                    | Instruction::Eval { .. }
                    | Instruction::CallMethod(_)
                    | Instruction::TailCallMethod(_)
                    | Instruction::Construct(_)
                    | Instruction::ConstructSuper(_)
                    | Instruction::InitDerivedConstructor
                    | Instruction::Apply(_)
                    | Instruction::ApplySuper
                    | Instruction::ApplyEval { .. }
            ) {
                if let Some(completion) = self.execute_call_instruction(instruction, host)? {
                    return Ok(InterpreterExit::Complete(completion));
                }
                continue;
            }

            if matches!(
                instruction,
                Instruction::Neg
                    | Instruction::Plus
                    | Instruction::Inc
                    | Instruction::Dec
                    | Instruction::PostInc
                    | Instruction::PostDec
                    | Instruction::BitNot
                    | Instruction::Not
                    | Instruction::TypeOf
                    | Instruction::IsUndefinedOrNull
                    | Instruction::IsUndefined
                    | Instruction::IsNull
                    | Instruction::TypeOfIsUndefined
                    | Instruction::TypeOfIsFunction
                    | Instruction::Add
                    | Instruction::Sub
                    | Instruction::Mul
                    | Instruction::Div
                    | Instruction::Mod
                    | Instruction::Pow
                    | Instruction::Shl
                    | Instruction::Sar
                    | Instruction::Shr
                    | Instruction::BitAnd
                    | Instruction::BitXor
                    | Instruction::BitOr
                    | Instruction::Eq
                    | Instruction::StrictEq
                    | Instruction::Neq
                    | Instruction::StrictNeq
                    | Instruction::Lt
                    | Instruction::Lte
                    | Instruction::Gt
                    | Instruction::Gte
            ) {
                if let Some(completion) = self.execute_numeric_instruction(instruction, host)? {
                    return Ok(InterpreterExit::Complete(completion));
                }
                continue;
            }

            if let Some(completion) = self.execute_hot_instruction(code, instruction, host)? {
                return Ok(InterpreterExit::Complete(completion));
            }
        }
    }

    pub(in crate::engine::vm) fn pop(&mut self) -> Result<Value, Error> {
        self.stack
            .pop()
            .ok_or_else(|| Error::internal("bytecode stack underflow"))
    }

    pub(in crate::engine::vm) fn clone_at_depth(&self, depth: u8) -> Result<Value, Error> {
        let index = self
            .stack
            .len()
            .checked_sub(usize::from(depth) + 1)
            .ok_or_else(|| Error::internal("bytecode stack depth operand is out of bounds"))?;
        self.stack
            .get(index)
            .cloned()
            .ok_or_else(|| Error::internal("bytecode stack depth operand is out of bounds"))
    }

    pub(in crate::engine::vm) fn take_call_arguments(
        &mut self,
        argument_count: u16,
        fixed_values: usize,
    ) -> Result<Vec<Value>, Error> {
        let argument_count = usize::from(argument_count);
        let required = argument_count
            .checked_add(fixed_values)
            .ok_or_else(|| Error::internal("call operand count overflow"))?;
        if self.stack.len() < required {
            return Err(Error::internal("call operands underflow the VM stack"));
        }
        let start = self.stack.len() - argument_count;
        Ok(self.stack.split_off(start))
    }

    pub(in crate::engine::vm) fn normalized_this(
        &mut self,
        host: &mut impl VmHost,
    ) -> Result<Value, Error> {
        if let Some(value) = &self.normalized_this {
            return Ok(value.clone());
        }
        if self.strict || matches!(self.this_value, Value::Object(_)) {
            return Ok(self.this_value.clone());
        }
        if matches!(self.this_value, Value::Undefined | Value::Null) {
            return self
                .callee_global
                .as_ref()
                .cloned()
                .map(Value::Object)
                .ok_or_else(|| Error::internal("sloppy frame has no callee global object"));
        }
        let value = host.box_primitive(self.this_value.clone())?;
        self.normalized_this = Some(value.clone());
        Ok(value)
    }

    pub(in crate::engine::vm) fn pop_pair(&mut self) -> Result<(Value, Value), Error> {
        let right = self.pop()?;
        let left = self.pop()?;
        Ok((left, right))
    }
}
