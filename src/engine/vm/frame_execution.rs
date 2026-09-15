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
        layout: crate::engine::code::function::layout::FrameLayout<'_>,
        caller_realm: ContextId,
        callee_realm: ContextId,
        current_function: ObjectRef,
        this_value: Value,
        new_target: Value,
        callee_global: ObjectRef,
    ) -> Self {
        Self {
            stack: Vec::with_capacity(layout.operand_capacity()),
            regions: Vec::new(),
            pc: 0,
            caller_realm: Some(caller_realm),
            callee_realm: Some(callee_realm),
            current_function: Some(current_function),
            this_value,
            normalized_this: None,
            new_target,
            strict: layout.is_strict(),
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
            #[cfg(feature = "profiling")]
            crate::engine::api::profiling::record_legacy_dispatch(self.stack.len());
            host.update_active_bytecode_pc(BytecodePc::new(self.pc))?;
            #[cfg(feature = "profiling")]
            crate::engine::api::profiling::record_legacy_pc_publication();
            self.pc = self
                .pc
                .checked_add(1)
                .ok_or_else(|| Error::internal("program counter overflow"))?;

            // Frame-local operations finish here without a second opcode match.
            // Larger semantic handlers remain separate to bound recursive
            // native frames. Every route publishes PC before executing.
            let completion = match instruction {
                Instruction::PushI32(value) => {
                    self.stack.push(Value::Int(*value));
                    continue;
                }
                Instruction::Undefined => {
                    self.stack.push(Value::Undefined);
                    continue;
                }
                Instruction::Null => {
                    self.stack.push(Value::Null);
                    continue;
                }
                Instruction::PushFalse => {
                    self.stack.push(Value::Bool(false));
                    continue;
                }
                Instruction::PushTrue => {
                    self.stack.push(Value::Bool(true));
                    continue;
                }
                Instruction::GetLocal(index) => {
                    self.stack.push(host.get_local(*index)?);
                    continue;
                }
                Instruction::PutLocal(index) => {
                    let value = self.pop()?;
                    host.put_local(*index, value)?;
                    continue;
                }
                Instruction::SetLocal(index) => {
                    let value = self
                        .stack
                        .last()
                        .cloned()
                        .ok_or_else(|| Error::internal("set local on an empty stack"))?;
                    host.put_local(*index, value)?;
                    continue;
                }
                Instruction::GetLocalCheck(index) => {
                    self.stack.push(host.get_local_checked(*index)?);
                    continue;
                }
                Instruction::PutLocalCheck(index) => {
                    let value = self.pop()?;
                    host.put_local_checked(*index, value)?;
                    continue;
                }
                Instruction::SetLocalCheck(index) => {
                    let value =
                        self.stack.last().cloned().ok_or_else(|| {
                            Error::internal("set lexical local on an empty stack")
                        })?;
                    host.put_local_checked(*index, value)?;
                    continue;
                }
                Instruction::GetArg(index) => {
                    self.stack.push(host.get_argument(*index)?);
                    continue;
                }
                Instruction::PutArg(index) => {
                    let value = self.pop()?;
                    host.put_argument(*index, value)?;
                    continue;
                }
                Instruction::SetArg(index) => {
                    let value = self
                        .stack
                        .last()
                        .cloned()
                        .ok_or_else(|| Error::internal("set argument on an empty stack"))?;
                    host.put_argument(*index, value)?;
                    continue;
                }
                Instruction::GetVarRef(index) => {
                    self.stack.push(host.get_var_ref(*index)?);
                    continue;
                }
                Instruction::PutVarRef(index) => {
                    let value = self.pop()?;
                    host.put_var_ref(*index, value)?;
                    continue;
                }
                Instruction::SetVarRef(index) => {
                    let value = self
                        .stack
                        .last()
                        .cloned()
                        .ok_or_else(|| Error::internal("set VarRef on an empty stack"))?;
                    host.put_var_ref(*index, value)?;
                    continue;
                }
                Instruction::GetVarRefCheck(index) => {
                    self.stack.push(host.get_var_ref_checked(*index)?);
                    continue;
                }
                Instruction::PutVarRefCheck(index) => {
                    let value = self.pop()?;
                    host.put_var_ref_checked(*index, value)?;
                    continue;
                }
                Instruction::Drop => {
                    self.pop()?;
                    continue;
                }
                Instruction::Dup => {
                    let value = self
                        .stack
                        .last()
                        .cloned()
                        .ok_or_else(|| Error::internal("dup on an empty stack"))?;
                    self.stack.push(value);
                    continue;
                }
                Instruction::Nip => {
                    let (_, value) = self.pop_pair()?;
                    self.stack.push(value);
                    continue;
                }
                Instruction::Swap => {
                    let (left, right) = self.pop_pair()?;
                    self.stack.push(right);
                    self.stack.push(left);
                    continue;
                }
                Instruction::IfFalse(target) => {
                    let value = self.pop()?;
                    if !host.to_boolean(&value)? {
                        self.pc = host.static_branch_target(*target, code.len())?;
                    }
                    continue;
                }
                Instruction::IfTrue(target) => {
                    let value = self.pop()?;
                    if host.to_boolean(&value)? {
                        self.pc = host.static_branch_target(*target, code.len())?;
                    }
                    continue;
                }
                Instruction::Goto(target) => {
                    self.pc = host.static_branch_target(*target, code.len())?;
                    continue;
                }
                Instruction::InitialYield => {
                    return Ok(InterpreterExit::Suspend(VmSuspendKind::Initial));
                }
                Instruction::Yield => return Ok(InterpreterExit::Suspend(VmSuspendKind::Yield)),
                Instruction::YieldStar => {
                    return Ok(InterpreterExit::Suspend(VmSuspendKind::YieldStar));
                }
                Instruction::AsyncYieldStar => {
                    return Ok(InterpreterExit::Suspend(VmSuspendKind::AsyncYieldStar));
                }
                Instruction::Await => return Ok(InterpreterExit::Suspend(VmSuspendKind::Await)),
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
                | Instruction::ForInNext => self.execute_cold_instruction(instruction, host)?,
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
                | Instruction::ApplyEval { .. } => {
                    self.execute_call_instruction(instruction, host)?
                }
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
                | Instruction::Gte => self.execute_numeric_instruction(instruction, host)?,
                _ => self.execute_hot_instruction(code, instruction, host)?,
            };
            if let Some(completion) = completion {
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
        // A too-large depth wraps above len, so the single checked lookup
        // rejects it. Valid depths select the original tail-relative slot.
        let index = self.stack.len().wrapping_sub(usize::from(depth) + 1);
        self.stack
            .get(index)
            .cloned()
            .ok_or_else(|| Error::internal("bytecode stack depth operand is out of bounds"))
    }

    /// Reuse this activation's argument tail for ordinary calls. Callee and
    /// receiver move out once; the argument values remain rooted in place.
    /// Always detach consumed operands after the synchronous host entry,
    /// including throws/internal errors, before the interpreter can unwind.
    pub(in crate::engine::vm) fn call_from_stack(
        &mut self,
        argument_count: u16,
        method: bool,
        host: &mut impl VmHost,
    ) -> Result<Completion, Error> {
        let fixed_values = if method { 2 } else { 1 };
        let required = usize::from(argument_count)
            .checked_add(fixed_values)
            .ok_or_else(|| Error::internal("call operand count overflow"))?;
        let base = self
            .stack
            .len()
            .checked_sub(required)
            .ok_or_else(|| Error::internal("call operands underflow the VM stack"))?;
        let arguments_start = base + fixed_values;
        let function = std::mem::replace(&mut self.stack[arguments_start - 1], Value::Undefined);
        let receiver = if method {
            std::mem::replace(&mut self.stack[base], Value::Undefined)
        } else {
            Value::Undefined
        };
        let result =
            host.call_with_borrowed_arguments(function, receiver, &self.stack[arguments_start..]);
        self.stack.truncate(base);
        result
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
        if self.stack.len() < 2 {
            // Match sequential-pop failure: consume a lone right operand,
            // construct the error, then release the operand.
            let right = self.pop()?;
            let error = Error::internal("bytecode stack underflow");
            drop(right);
            return Err(error);
        }
        // The shared bound proves both pops. Neither move can invoke user
        // code or change the stack except by removing its own operand.
        let right = self.stack.pop().expect("two operands were checked");
        let left = self.stack.pop().expect("one checked operand remains");
        Ok((left, right))
    }
}
