"""Pinned data and expected shapes for publication."""

EXPECTED_ORDINARY_VERIFIER_ENTRY = ('pub(in crate::engine::code) fn verify_unlinked_ordinary_leaf( function: &UnlinkedFunction, ) -> '
 'Result<(), RuntimeError> { verify_unlinked_tree_with_root(function, '
 'RootPublication::TrustedOrdinaryLeaf) }')

EXPECTED_ORDINARY_PUBLIC_API = ('pub fn read_trusted_ordinary_function( &mut self, bytes: &[u8], root_constant_index: u32, ) -> '
 'Result<CallableRef, RuntimeError> { let result = '
 'self.runtime.read_trusted_ordinary_function_in_realm( self.realm, bytes, root_constant_index, ); '
 'self.finish_trusted_bytecode_read(result) }')

PREDICATE_REQUIREMENTS = {'IsUndefinedOrNull': ('matches!(value, Value::Undefined | Value::Null)', False),
 'IsUndefined': ('matches!(value, Value::Undefined)', False),
 'IsNull': ('matches!(value, Value::Null)', False),
 'TypeOfIsUndefined': ('matches!(value, Value::Undefined) || host.is_html_dda(&value)?', True),
 'TypeOfIsFunction': ('!host.is_html_dda(&value)? && host.is_callable(&value)?', True)}
