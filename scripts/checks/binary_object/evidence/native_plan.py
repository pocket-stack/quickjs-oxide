"""Pinned data and expected shapes for native_plan."""

EXPECTED_NATIVE_PLAN_VISIBLE_ITEMS = [('enum', 'NativeAtomClass'),
 ('struct', 'NativeAtomRef'),
 ('fn', 'originates_from_input_atom_table'),
 ('fn', 'class'),
 ('fn', 'index'),
 ('fn', 'manifest_string'),
 ('fn', 'dynamic_string'),
 ('fn', 'identity_description'),
 ('struct', 'NativeLabel'),
 ('fn', 'operand_pc'),
 ('fn', 'displacement'),
 ('fn', 'target_pc'),
 ('fn', 'target_instruction'),
 ('enum', 'NativeOperands'),
 ('fn', 'format'),
 ('struct', 'NativeInstruction'),
 ('fn', 'byte_pc'),
 ('fn', 'opcode'),
 ('fn', 'operands'),
 ('struct', 'NativeCodePlan'),
 ('fn', 'function'),
 ('fn', 'instructions'),
 ('fn', 'native_pc_map'),
 ('fn', 'instruction_at_native_pc'),
 ('enum', 'NativePlanError'),
 ('fn', 'is_label_target_error'),
 ('fn', 'decode_native_code_plan')]

EXPECTED_NATIVE_PLAN_USES = {'use crate::engine::code::binary_object::pinned_atoms::{FIRST_DYNAMIC_ATOM, PinnedAtomKind};',
 'use crate::engine::code::binary_object::pinned_opcodes::{OpcodeFormat, PinnedOpcode};',
 'use crate::engine::code::binary_object::wire::WireString;',
 'use std::fmt;',
 'use super::{BytecodeImage, FunctionId, ImageAtom, ImageCode};'}

EXPECTED_NATIVE_PLAN_TYPE_ITEMS = [('enum', 'NativeAtomClass'),
 ('struct', 'NativeAtomRef'),
 ('enum', 'NativeAtomRefKind'),
 ('struct', 'NativeLabel'),
 ('enum', 'NativeOperands'),
 ('struct', 'NativeInstruction'),
 ('struct', 'NativeCodePlan'),
 ('enum', 'NativePlanError'),
 ('struct', 'DecodedCodePlan')]

EXPECTED_NATIVE_PLAN_FUNCTION_NAMES = ['new',
 'originates_from_input_atom_table',
 'class',
 'index',
 'manifest_string',
 'dynamic_string',
 'identity_description',
 'operand_pc',
 'displacement',
 'target_pc',
 'target_instruction',
 'format',
 'byte_pc',
 'opcode',
 'operands',
 'function',
 'instructions',
 'native_pc_map',
 'instruction_at_native_pc',
 'is_label_target_error',
 'fmt',
 'decode_native_code_plan',
 'decode_code_plan',
 'validate_instruction_boundaries',
 'decode_operands',
 'implicit_integer',
 'implicit_slot',
 'invalid_implicit',
 'decode_label',
 'format_size',
 'read_u8',
 'read_i8',
 'read_u16',
 'read_i16',
 'read_u32',
 'read_i32',
 'read_array',
 'truncated_operand']

EXPECTED_NATIVE_ATOM_REF_KIND_SOURCE = ('\n'
 "    enum NativeAtomRefKind<'image> {\n"
 '        Null,\n'
 '        Index(u32),\n'
 '        Manifest {\n'
 '            class: NativeAtomClass,\n'
 "            spelling: &'static str,\n"
 '        },\n'
 "        Dynamic(&'image WireString),\n"
 '    }\n')
