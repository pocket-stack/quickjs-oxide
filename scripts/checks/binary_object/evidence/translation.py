"""Pinned data and expected shapes for translation."""

EXPECTED_STAGE_BOUNDARIES = {8: (('push_this', 1, 0, 1, 'None'), 'OrdinaryOnly', 'Recipe::PushThis'),
 11: (('object', 1, 0, 1, 'None'), 'OrdinaryOnly', 'Recipe::Object'),
 33: (('call_constructor', 3, 2, 1, 'NPop'), 'OrdinaryOnly', 'Recipe::Construct'),
 34: (('call', 3, 1, 1, 'NPop'), 'OrdinaryOnly', 'Recipe::Call'),
 35: (('tail_call', 3, 1, 0, 'NPop'), 'OrdinaryOnly', 'Recipe::TailCall'),
 36: (('call_method', 3, 2, 1, 'NPop'), 'OrdinaryOnly', 'Recipe::CallMethod'),
 37: (('tail_call_method', 3, 2, 0, 'NPop'), 'OrdinaryOnly', 'Recipe::TailCallMethod'),
 38: (('array_from', 3, 0, 1, 'NPop'), 'OrdinaryOnly', 'Recipe::ArrayFrom'),
 39: (('apply', 3, 3, 1, 'U16'), 'OrdinaryOnly', 'Recipe::Apply'),
 41: (('return_undef', 1, 0, 0, 'None'), 'OrdinaryOnly', 'Recipe::ReturnUndefined'),
 48: (('throw', 1, 1, 0, 'None'), 'OrdinaryOnly', 'Recipe::Throw'),
 49: (('throw_error', 6, 0, 0, 'AtomU8'), 'OrdinaryOnly', 'Recipe::ThrowReadOnly'),
 111: (('to_object', 1, 1, 1, 'None'), 'OrdinaryOnly', 'Recipe::ToObject'),
 112: (('to_propkey', 1, 1, 1, 'None'), 'OrdinaryOnly', 'Recipe::ToPropKey'),
 177: (('nop', 1, 0, 0, 'None'), 'OrdinaryOnly', 'Recipe::Nop'),
 236: (('call0', 1, 1, 1, 'NPopX'), 'OrdinaryOnly', 'Recipe::Call'),
 237: (('call1', 1, 1, 1, 'NPopX'), 'OrdinaryOnly', 'Recipe::Call'),
 238: (('call2', 1, 1, 1, 'NPopX'), 'OrdinaryOnly', 'Recipe::Call'),
 239: (('call3', 1, 1, 1, 'NPopX'), 'OrdinaryOnly', 'Recipe::Call')}

STAGE_ONE_ORDINARY_ROWS = (6,
 7,
 9,
 10,
 14,
 15,
 16,
 17,
 18,
 19,
 20,
 21,
 22,
 23,
 24,
 25,
 26,
 27,
 28,
 29,
 30,
 31,
 32,
 41,
 105,
 138,
 139,
 140,
 141,
 142,
 143,
 147,
 148,
 149,
 152,
 154,
 157,
 158,
 159,
 160,
 161,
 162,
 164,
 167,
 168,
 170,
 171,
 172,
 173,
 174,
 176,
 191,
 233,
 240,
 241,
 242,
 243)

EXPECTED_INSTRUCTION_NEW = ('pub(super) const fn new( audience: InstructionAudience, diagnostic: OperationDiagnostic, '
 "operation: FunctionOp<'image>, ) -> Self { Self { audience, diagnostic, operation, } }")

FORBIDDEN_DTO_TRAITS = {'AtomStringSpelling': {'Hash', 'Eq', 'PartialEq'},
 'AtomOperandValue': {'Hash', 'Eq', 'PartialEq'},
 'AtomOperand': {'Hash', 'Eq', 'PartialEq'},
 'FunctionOp': {'Hash', 'Eq', 'PartialEq'},
 'FunctionInstruction': {'Hash', 'Eq', 'PartialEq', 'Debug'},
 'FunctionCode': {'Hash', 'Eq', 'Debug', 'PartialEq', 'Default'},
 'OperationDiagnostic': {'Hash', 'Debug'}}

EXPECTED_PENDING_INITIALIZERS = [('Some(operation), None, None, None', '1'),
 ('Some(first), Some(second), None, None', '2'),
 ('Some(first), Some(second), Some(third), None', '3'),
 ('Some(first), Some(second), Some(third), Some(fourth)', '4')]

PENDING_HELPER_FRAGMENTS = ('const fn len(&self) -> usize { self.len as usize }',
 "fn into_operations(self) -> impl Iterator<Item = PendingOperation<'image>> { self.operations "
 '.into_iter() .take(usize::from(self.len)) .flatten() }')

APPLY_MAGIC_ERROR_CONTRACTS = (('non_canonical_apply_magic',
  'fn non_canonical_apply_magic(magic: u16) -> Self { Self { kind: '
  'FunctionTranslateErrorKind::NonCanonicalApplyMagic(magic), } }'),
 ('unadmitted_throw_error_subtype',
  'fn unadmitted_throw_error_subtype(subtype: u8) -> Self { Self { kind: '
  'FunctionTranslateErrorKind::UnadmittedThrowErrorSubtype(subtype), } }'),
 ('is_unadmitted_operand_error',
  'fn is_unadmitted_operand_error(&self) -> bool { matches!( self.kind, '
  'FunctionTranslateErrorKind::NonCanonicalApplyMagic(_) | '
  'FunctionTranslateErrorKind::UnadmittedThrowErrorSubtype(_) ) }'))

EXPECTED_STACK_EXPANSIONS = {'Nip1': ('two', ('Perm3', 'Nip')),
 'Dup2': ('three', ('Dup1', 'Dup', 'Perm3')),
 'Swap2': ('two', ('Rot4Left', 'Rot4Left')),
 'Rot3Left': ('two', ('Perm3', 'Swap')),
 'Rot3Right': ('two', ('Swap', 'Perm3')),
 'Rot5Left': ('four', ('Perm4', 'Perm4', 'Perm5', 'Rot4Left'))}

DIAGNOSTIC_FRAGMENTS = ('FunctionTranslateError::registry_drift( opcode.name(), expected_shape, descriptor_shape, '
 'decoded_shape, )',
 'let diagnostic = OperationDiagnostic::new(opcode.name(), expected_shape);')

BRANCH_FRAGMENTS = ('source_to_output.push(output_index);',
 'PendingOperation::Ready(operation) => operation,',
 'PendingOperation::IfFalse(target) => { FunctionOp::IfFalse(resolve_target(&source_to_output, '
 'target)?) }',
 'PendingOperation::IfTrue(target) => { FunctionOp::IfTrue(resolve_target(&source_to_output, '
 'target)?) }',
 'PendingOperation::Goto(target) => { FunctionOp::Goto(resolve_target(&source_to_output, target)?) '
 '}',
 'output.push(FunctionInstruction::new( instruction.audience, instruction.diagnostic, operation, '
 '));')

SOURCE_MAP_FRAGMENTS = ('let mut output_len = 0_usize;',
 'let output_index = u32::try_from(output_len)',
 'source_to_output.push(output_index);',
 'let (audience, expansion) = match row.policy',
 'output_len = output_len .checked_add(expansion.len())',
 'pending.push(PendingInstruction')

EXPECTED_RESOLVE_TARGET = ('fn resolve_target( source_to_output: &[u32], target_instruction: u32, ) -> Result<u32, '
 'FunctionTranslateError> { source_to_output .get(target_instruction as usize) .copied() '
 '.ok_or_else(FunctionTranslateError::invalid_branch_target) }')
