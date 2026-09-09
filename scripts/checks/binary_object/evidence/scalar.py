"""Pinned data and expected shapes for scalar."""

EXPECTED_SCALAR_TOP_LEVEL_FUNCTIONS = ['decode_trusted_scalar_script',
 'admit_image',
 'project_atom_string',
 'project_atom_string_spelling',
 'classify_translation_error',
 'copy_wire_string',
 'copy_utf16',
 'copy_bigint_bytes',
 'decode_scalar_sequence',
 'decode_scalar_push',
 'decode_direct_scalar_push',
 'unadmitted',
 'classify_image_error',
 'classify_atom_error',
 'classify_wire_error',
 'classify_data_error',
 'classify_envelope_error',
 'classify_code_error']

SCALAR_ADMISSION_FRAGMENTS = ('let translated = translate_function(image, root, TranslationTarget::Scalar) '
 '.map_err(classify_translation_error)?;',
 'let Some(sequence) = decode_scalar_sequence(translated)? else',
 'if !matches!(&sequence.push, ScalarPush::AtomValue(_)) && image.input_atom_slot_count() != 0',
 'let ScalarSequence { push, unary_ops } = sequence;',
 'let value = match (push, function.constants())',
 '(ScalarPush::AtomValue(atom), []) => project_atom_string(image, atom)?',
 '}; Ok((value, unary_ops))')

ATOM_PROJECTION_FRAGMENTS = ('0 if atom.originates_from_input_atom_table() =>',
 '1 if !atom.originates_from_input_atom_table() =>',
 'AtomOperandClass::Null => unadmitted(',
 'AtomOperandClass::Private => unadmitted(',
 'AtomOperandClass::Symbol => unadmitted(',
 '.index_value() .map(ScalarValueDraft::IntegerAtomString)',
 'AtomOperandClass::String => project_atom_string_spelling(atom)')

ATOM_SPELLING_FRAGMENTS = ('let Some(length) = atom.string_utf16_len() else',
 'let Some(units) = atom.string_utf16_units() else',
 'copy_utf16(units, length).map(ScalarValueDraft::AtomString)')
