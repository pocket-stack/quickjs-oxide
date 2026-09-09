"""Pinned data and expected shapes for runtime_protocols."""

VERIFIER_TERMINAL_GUARD = ('if matches!( instruction, Instruction::TailCall(_) | Instruction::TailCallMethod(_) | '
 'Instruction::Return | Instruction::ReturnUndefined | Instruction::ReturnDerived(_) ) && state')

TAIL_TERMINAL_DISPATCH = ('Instruction::TailCall(_) | Instruction::TailCallMethod(_) | Instruction::Return | '
 'Instruction::ReturnUndefined | Instruction::ReturnDerived(_) | Instruction::Throw | '
 'Instruction::Ret => {}')

EXPECTED_TAIL_CALL_ARM = ('Instruction::TailCall(argument_count) => { let arguments = '
 'self.take_call_arguments(*argument_count, 1)?; let function = self.pop()?; return '
 'host.call(function, Value::Undefined, arguments).map(Some); }')

EXPECTED_TAIL_METHOD_ARM = ('Instruction::TailCallMethod(argument_count) => { let arguments = '
 'self.take_call_arguments(*argument_count, 2)?; let function = self.pop()?; let receiver = '
 'self.pop()?; return host.call(function, receiver, arguments).map(Some); }')

CALL_ROUTE = ('if matches!( instruction, Instruction::Import | Instruction::Call(_) | Instruction::TailCall(_) '
 '| Instruction::Eval { .. } | Instruction::CallMethod(_) | Instruction::TailCallMethod(_) | '
 'Instruction::Construct(_) | Instruction::ConstructSuper(_) | Instruction::InitDerivedConstructor '
 '| Instruction::Apply(_) | Instruction::ApplySuper | Instruction::ApplyEval { .. } ) { if let '
 'Some(completion) = self.execute_call_instruction(instruction, host)? { return '
 'Ok(InterpreterExit::Complete(completion)); } continue; }')
