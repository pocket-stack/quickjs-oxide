"""Pinned data and expected shapes for ordinary_leaf."""

APPLY_ADMISSION_FRAGMENTS = ('if error.is_label_target_error() { return OrdinaryLeafReadError::Unadmitted(',
 'if error.is_unadmitted_operand_error() { return '
 'OrdinaryLeafReadError::Unadmitted(error.to_string()); }',
 'let message = error.to_string();',
 'OrdinaryLeafReadError::Internal(message)')

SCALAR_SEQUENCE_FRAGMENTS = ('.any(|instruction| !instruction.supports_scalar())',
 '!matches!(set_completion.operation(), FunctionOp::SetLocal(0))',
 '!matches!(return_value.operation(), FunctionOp::Return)',
 'let FunctionOp::Unary(operation) = instruction.operation() else',
 'unary_ops.push(ScalarUnaryOp::from_translated(*operation));',
 '.and_then(|instruction| decode_scalar_push(instruction.into_operation()))')

SCALAR_STRING_FRAGMENTS = ('pub(in crate::engine::code) fn into_units(self) -> Box<[u16]> { self.0 }',
 'WireString::Narrow(bytes) => { copy_utf16(bytes.iter().copied().map(u16::from), bytes.len()) }',
 'WireString::Wide(units) => copy_utf16(units.iter().copied(), units.len()),')

EXPECTED_SCALAR_VISIBLE_ITEMS = [('pub(in crate::engine::code)', 'enum', 'ScalarScriptReadError'),
 ('pub(in crate::engine::code)', 'enum', 'ScalarUnaryOp'),
 ('pub(in crate::engine::code)', 'enum', 'ScalarValueDraft'),
 ('pub(in crate::engine::code)', 'struct', 'ScalarStringDraft'),
 ('pub(in crate::engine::code)', 'fn', 'into_units'),
 ('pub(in crate::engine::code)', 'fn', 'decode_trusted_scalar_script')]

EXPECTED_SCALAR_TOP_LEVEL_ITEMS = [('enum', 'ScalarValueDraft'),
 ('enum', 'ScalarUnaryOp'),
 ('struct', 'ScalarStringDraft'),
 ('enum', 'ScalarPush'),
 ('struct', 'ScalarSequence'),
 ('enum', 'ScalarScriptReadError'),
 ('struct', 'AdmissionLimits')]
