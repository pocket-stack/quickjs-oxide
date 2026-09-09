"""Pinned data and expected shapes for source_ownership."""

EXPECTED_MODEL_BYTECODE_IMAGE_METHODS = [('pub(super)', 'new'),
 ('pub(in crate::engine::code::binary_object)', 'input_atom_slot_count'),
 ('pub(in crate::engine::code)', 'atoms'),
 ('pub(super)', 'nodes'),
 ('pub(in crate::engine::code::binary_object)', 'sab_archive_occurrences'),
 ('pub(in crate::engine::code)', 'reference_table'),
 ('pub(in crate::engine::code)', 'functions'),
 ('pub(in crate::engine::code)', 'function'),
 ('pub(in crate::engine::code)', 'modules'),
 ('pub(in crate::engine::code)', 'module'),
 ('pub(in crate::engine::code)', 'root')]

EXPECTED_ATOM_SENSITIVE_VISIBLE_SITES = [('src/engine/code/binary_object/bytecode_image/model.rs',
  'pub(in crate::engine::code::binary_object)',
  'name_is_null'),
 ('src/engine/code/binary_object/bytecode_image/model.rs',
  'pub(in crate::engine::code::binary_object)',
  'name_is_pinned_eval')]
