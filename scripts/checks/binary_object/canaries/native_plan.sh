expect_rejected native-plan-tuple-raw-atom native-plan-type-set \
    src/engine/code/binary_object/bytecode_image/native_plan.rs \
    'struct HiddenRawAtom(ImageAtom);'
expect_rejected native-plan-private-raw-helper native-plan-function-set \
    src/engine/code/binary_object/bytecode_image/native_plan.rs \
    'fn leaked_raw_atom(atom: ImageAtom) -> ImageAtom { atom }'
expect_rejected native-plan-unicode-raw-helper native-plan-function-set \
    src/engine/code/binary_object/bytecode_image/native_plan.rs \
    'fn 泄漏(atom: ImageAtom) -> ImageAtom { atom }'
expect_rejected native-plan-unicode-type-alias native-plan-expansion \
    src/engine/code/binary_object/bytecode_image/native_plan.rs \
    'type 泄漏 = ImageAtom;'
expect_rejected native-plan-unicode-module native-plan-expansion \
    src/engine/code/binary_object/bytecode_image/native_plan.rs \
    'mod 泄漏 {}'
expect_rejected native-plan-const-raw-helper native-plan-data-item-set \
    src/engine/code/binary_object/bytecode_image/native_plan.rs \
    'const RAW_CODE: for<'"'"'a> fn(&'"'"'a ImageCode) -> &'"'"'a [u8] = |code| code.as_bytes();'
expect_rewrite_rejected native-plan-raw-byte-storage native-plan-facade-representation \
    src/engine/code/binary_object/bytecode_image/native_plan.rs \
    $'struct DecodedCodePlan<\'image> {\n    instructions: Box<[NativeInstruction<\'image>]>,' \
    $'struct DecodedCodePlan<\'image> {\n    raw_bytes: &\'image [u8],\n    instructions: Box<[NativeInstruction<\'image>]>,'
expect_rewrite_rejected native-plan-runtime-string-storage native-plan-facade-representation \
    src/engine/code/binary_object/bytecode_image/native_plan.rs \
    $'struct DecodedCodePlan<\'image> {\n    instructions: Box<[NativeInstruction<\'image>]>,' \
    $'struct DecodedCodePlan<\'image> {\n    runtime_string: JsString,\n    instructions: Box<[NativeInstruction<\'image>]>,'
expect_rewrite_rejected native-plan-module-escape native-plan-expansion \
    src/engine/code/binary_object/bytecode_image/native_plan.rs \
    'use std::fmt;' \
    $'use std::fmt;\nmod escape {}'
expect_rewrite_rejected native-plan-include-escape native-plan-expansion \
    src/engine/code/binary_object/bytecode_image/native_plan.rs \
    'use std::fmt;' \
    $'use std::fmt;\ninclude!("native_plan_escape.rs");'
expect_rewrite_rejected native-plan-trait-escape native-plan-expansion \
    src/engine/code/binary_object/bytecode_image/native_plan.rs \
    'use std::fmt;' \
    $'use std::fmt;\ntrait NativePlanEscape {}'
expect_rejected native-plan-sibling-consumer native-plan-consumer-set \
    src/engine/code/binary_object/scalar_script.rs \
    'use super::bytecode_image::native_plan::NativeCodePlan;'
expect_rejected native-plan-second-consumer native-plan-consumer-set \
    src/engine/code/binary_object/atoms.rs \
    'fn consume_native_plan(_: NativeCodePlan) {}'
expect_rejected native-plan-facade native-plan-private-stage \
    src/engine/code/binary_object/bytecode_image/mod.rs \
    'pub(in crate::engine::code::binary_object) use native_plan::NativeCodePlan;'
expect_rewrite_rejected native-plan-atom-class-collapse native-plan-semantic-seal \
    src/engine/code/binary_object/bytecode_image/native_plan.rs \
    $'                    PinnedAtomKind::String => NativeAtomClass::String,\n                    PinnedAtomKind::Private => NativeAtomClass::Private,\n                    PinnedAtomKind::Symbol => NativeAtomClass::Symbol,' \
    $'                    PinnedAtomKind::String => NativeAtomClass::String,\n                    PinnedAtomKind::Private => NativeAtomClass::String,\n                    PinnedAtomKind::Symbol => NativeAtomClass::String,'
expect_rewrite_rejected native-plan-raw-pinned-helper native-plan-semantic-seal \
    src/engine/code/binary_object/bytecode_image/native_plan.rs \
    '                spelling: atom.spelling(),' \
    '                spelling: if atom.raw() == 0 { atom.spelling() } else { atom.spelling() },'
expect_rewrite_rejected native-plan-dynamic-index-helper native-plan-semantic-seal \
    src/engine/code/binary_object/bytecode_image/native_plan.rs \
    'dynamic_atoms.get(index.as_usize())' \
    'dynamic_atoms.get(index.zero_based() as usize)'
expect_rewrite_rejected native-plan-origin-accessor-drift native-plan-semantic-seal \
    src/engine/code/binary_object/bytecode_image/native_plan.rs \
    $'    pub(in crate::engine::code::binary_object) const fn originates_from_input_atom_table(\n        self,\n    ) -> bool {\n        self.from_input_atom_table\n    }' \
    $'    pub(in crate::engine::code::binary_object) const fn originates_from_input_atom_table(\n        self,\n    ) -> bool {\n        false\n    }'
expect_rewrite_rejected native-plan-origin-range-widening native-plan-semantic-seal \
    src/engine/code/binary_object/bytecode_image/native_plan.rs \
    '.is_some_and(|slot| slot < input_atom_slot_count);' \
    '.is_some_and(|slot| slot <= input_atom_slot_count);'
expect_rewrite_rejected native-plan-label-error-broadening native-plan-semantic-seal \
    src/engine/code/binary_object/bytecode_image/native_plan.rs \
    'Self::LabelTargetOutOfRange { .. } | Self::LabelTargetNotInstructionBoundary { .. }' \
    'Self::LabelTargetOutOfRange { .. } | Self::LabelTargetNotInstructionBoundary { .. } | Self::InvalidOpcode { .. }'
expect_rewrite_rejected native-plan-label-error-collapse native-plan-semantic-seal \
    src/engine/code/binary_object/bytecode_image/native_plan.rs \
    'Self::LabelTargetOutOfRange { .. } | Self::LabelTargetNotInstructionBoundary { .. }' \
    'Self::LabelTargetOutOfRange { .. }'
expect_rewrite_rejected native-plan-label-accessor-drift native-plan-semantic-seal \
    src/engine/code/binary_object/bytecode_image/native_plan.rs \
    $'    pub(in crate::engine::code::binary_object) const fn target_pc(self) -> u32 {\n        self.target_pc\n    }' \
    $'    pub(in crate::engine::code::binary_object) const fn target_pc(self) -> u32 {\n        self.operand_pc\n    }'
expect_rewrite_rejected native-plan-label-base-drift native-plan-semantic-seal \
    src/engine/code/binary_object/bytecode_image/native_plan.rs \
    '.checked_add(u32::from(operand_offset))' \
    '.checked_add(1)'
expect_rewrite_rejected native-plan-format-size-drift native-plan-semantic-seal \
    src/engine/code/binary_object/bytecode_image/native_plan.rs \
    '        OpcodeFormat::AtomU8 => 6,' \
    '        OpcodeFormat::AtomU8 => 7,'
expect_rewrite_rejected native-plan-relocation-base-drift native-plan-semantic-seal \
    src/engine/code/binary_object/bytecode_image/native_plan.rs \
    $'            let expected = byte_pc\n                .checked_add(1)' \
    $'            let expected = byte_pc\n                .checked_add(0)'
expect_rewrite_rejected native-plan-atom-label-base-drift native-plan-semantic-seal \
    src/engine/code/binary_object/bytecode_image/native_plan.rs \
    $'            label: label32(5)?,\n            value: read_u8(instruction, 9, byte_pc, opcode)?,' \
    $'            label: label32(1)?,\n            value: read_u8(instruction, 9, byte_pc, opcode)?,'
expect_rewrite_rejected native-plan-implicit-minus-one-drift native-plan-implicit-opcode-set \
    src/engine/code/binary_object/bytecode_image/native_plan.rs \
    'opcode.name() == "push_minus1"' \
    'opcode.name() == "push_0"'
expect_rewrite_rejected native-plan-implicit-local-drift native-plan-implicit-opcode-set \
    src/engine/code/binary_object/bytecode_image/native_plan.rs \
    '&["get_loc", "put_loc", "set_loc"]' \
    '&["get_arg", "put_loc", "set_loc"]'
