expect_rewrite_rejected scalar-unary-chain-fold scalar-script-translated-code \
    src/engine/code/binary_object/scalar_script.rs \
    '        unary_ops.push(ScalarUnaryOp::from_translated(*operation));' \
    '        if unary_ops.last() != Some(&ScalarUnaryOp::from_translated(*operation)) { unary_ops.push(ScalarUnaryOp::from_translated(*operation)); }'
expect_rewrite_rejected scalar-unary-chain-reorder scalar-script-translated-code \
    src/engine/code/binary_object/scalar_script.rs \
    '        unary_ops.push(ScalarUnaryOp::from_translated(*operation));' \
    '        unary_ops.insert(0, ScalarUnaryOp::from_translated(*operation));'
expect_rewrite_rejected scalar-completion-slot-drift scalar-script-translated-code \
    src/engine/code/binary_object/scalar_script.rs \
    'FunctionOp::SetLocal(0)' \
    'FunctionOp::SetLocal(1)'
expect_rewrite_rejected scalar-return-kind-drift scalar-script-translated-code \
    src/engine/code/binary_object/scalar_script.rs \
    'FunctionOp::Return)' \
    'FunctionOp::OutsideTarget)'
expect_rewrite_rejected scalar-post-projection-early-rejection scalar-script-translated-admission \
    src/engine/code/binary_object/scalar_script.rs \
    '    Ok((value, unary_ops))' \
    $'    if false { return unadmitted("early rejection"); }\n    Ok((value, unary_ops))'
expect_rewrite_rejected scalar-bigint-infallible-copy scalar-script-bigint-copy \
    src/engine/code/binary_object/scalar_script.rs \
    'copy.try_reserve_exact(bytes.len())' \
    'copy.reserve(bytes.len())'
expect_rewrite_rejected scalar-string-utf8-misdecode scalar-script-string-copy \
    src/engine/code/binary_object/scalar_script.rs \
    'copy_utf16(bytes.iter().copied().map(u16::from), bytes.len())' \
    'copy_utf16(String::from_utf8_lossy(bytes).encode_utf16(), bytes.len())'
expect_rewrite_rejected scalar-constant-pairing-bypass scalar-script-translated-admission \
    src/engine/code/binary_object/scalar_script.rs \
    '    let value = match (push, function.constants()) {' \
    $'    let value = ScalarValueDraft::Float64Bits(0);\n    let _reviewed_pair = match (push, function.constants()) {'
expect_rewrite_rejected scalar-input-atom-slot-widening scalar-script-translated-admission \
    src/engine/code/binary_object/scalar_script.rs \
    'image.input_atom_slot_count() != 0' \
    'image.input_atom_slot_count() != 2'
expect_rewrite_rejected scalar-admission-early-success scalar-script-translated-admission \
    src/engine/code/binary_object/scalar_script.rs \
    '    let translated = translate_function(image, root, TranslationTarget::Scalar)' \
    '    return Ok((ScalarValueDraft::EmptyString, Box::default())); let translated = translate_function(image, root, TranslationTarget::Scalar)'
expect_rewrite_rejected scalar-label-error-bypass scalar-script-translated-admission \
    src/engine/code/binary_object/scalar_script.rs \
    '    if error.is_label_target_error() {' \
    '    if false && error.is_label_target_error() {'
expect_rewrite_rejected scalar-input-origin-zero-bypass scalar-native-atom-consumer \
    src/engine/code/binary_object/scalar_script.rs \
    '        0 if atom.originates_from_input_atom_table() => {' \
    '        0 if false && atom.originates_from_input_atom_table() => {'
expect_rewrite_rejected scalar-input-origin-one-bypass scalar-native-atom-consumer \
    src/engine/code/binary_object/scalar_script.rs \
    '        1 if !atom.originates_from_input_atom_table() => {' \
    '        1 if false && !atom.originates_from_input_atom_table() => {'
expect_rewrite_rejected scalar-private-identity-admission scalar-native-atom-consumer \
    src/engine/code/binary_object/scalar_script.rs \
    '        AtomOperandClass::Private => unadmitted("private atom is not a String value"),' \
    '        AtomOperandClass::Private => project_atom_string_spelling(atom),'
expect_rewrite_rejected scalar-symbol-identity-admission scalar-native-atom-consumer \
    src/engine/code/binary_object/scalar_script.rs \
    '        AtomOperandClass::Symbol => unadmitted("symbol atom is not a String value"),' \
    '        AtomOperandClass::Symbol => project_atom_string_spelling(atom),'
expect_rewrite_rejected scalar-index-identity-collapse scalar-native-atom-consumer \
    src/engine/code/binary_object/scalar_script.rs \
    '            .map(ScalarValueDraft::IntegerAtomString)' \
    '            .map(|_| ScalarValueDraft::IntegerAtomString(0))'
expect_rewrite_rejected scalar-error-missing-unadmitted scalar-script-error-shape \
    src/engine/code/binary_object/scalar_script.rs \
    '    Unadmitted(String),' \
    '    Rejected(String),'
expect_rejected scalar-extra-visible-item scalar-script-visible-item-set \
    src/engine/code/binary_object/scalar_script.rs \
    'pub(in crate::engine::code) fn leak_image() {}'
expect_rewrite_rejected scalar-unary-visibility-widening scalar-unary-operation-shape \
    src/engine/code/binary_object/scalar_script.rs \
    'pub(in crate::engine::code) enum ScalarUnaryOp {' \
    'pub(crate) enum ScalarUnaryOp {'
expect_rejected scalar-helper-escape scalar-script-helper-set \
    src/engine/code/binary_object/scalar_script.rs \
    'fn admit_unary_without_sidecars() {}'
expect_rewrite_rejected consumer-publication-visibility-widening binary-object-consumer-publication \
    src/engine/code/binary_object_publish.rs \
    '    pub(crate) fn read_trusted_scalar_script_in_realm(' \
    '    pub fn read_trusted_scalar_script_in_realm('
expect_full_rewrite_rejected consumer-source-include binary-object-consumer-source-include \
    src/engine/code/binary_object_publish.rs \
    $'#[cfg(test)]\nmod tests {' \
    $'include!("scalar_unary_escape.rs");\n\n#[cfg(test)]\nmod tests {'
expect_full_rewrite_rejected consumer-private-module binary-object-consumer-top-level-item-set \
    src/engine/code/binary_object_publish.rs \
    $'#[cfg(test)]\nmod tests {' \
    $'mod scalar_unary_escape;\n\n#[cfg(test)]\nmod tests {'
expect_full_rewrite_rejected consumer-private-trait binary-object-consumer-top-level-item-set \
    src/engine/code/binary_object_publish.rs \
    $'#[cfg(test)]\nmod tests {' \
    $'trait ScalarUnaryEscape {}\n\n#[cfg(test)]\nmod tests {'
expect_full_rewrite_rejected consumer-helper-escape binary-object-consumer-helper-set \
    src/engine/code/binary_object_publish.rs \
    $'#[cfg(test)]\nmod tests {' \
    $'fn publish_unary_without_verification() {}\n\n#[cfg(test)]\nmod tests {'
expect_full_rewrite_rejected consumer-macro-escape binary-object-consumer-macro-set \
    src/engine/code/binary_object_publish.rs \
    $'#[cfg(test)]\nmod tests {' \
    $'scalar_unary_escape!();\n\n#[cfg(test)]\nmod tests {'
expect_rewrite_rejected scalar-strict-reader scalar-script-reader-mode \
    src/engine/code/binary_object/scalar_script.rs \
    'ReaderMode::QuickJsCompatible' \
    'ReaderMode::Strict'
expect_rewrite_rejected image-atom-visibility-widening image-atom-visibility \
    src/engine/code/binary_object/bytecode_image/atoms.rs \
    'pub(super) enum ImageAtom {' \
    'pub(in crate::engine::code::binary_object) enum ImageAtom {'
expect_rejected image-atom-reexport image-atom-reexport \
    src/engine/code/binary_object/bytecode_image/mod.rs \
    'pub(in crate::engine::code::binary_object) use atoms::ImageAtom;'
expect_rejected image-atom-raw-accessor image-atom-escape \
    src/engine/code/binary_object/bytecode_image/model.rs \
    'impl ImageLocalVariable { pub(in crate::engine::code::binary_object) const fn raw_name(&self) -> ImageAtom { self.name } }'
expect_rejected bytecode-image-trait-raw-u32 bytecode-image-implementation-set \
    src/engine/code/binary_object/bytecode_image/model.rs \
    'impl From<&BytecodeImage> for Vec<u32> { fn from(image: &BytecodeImage) -> Self { image.functions().iter().flat_map(|function| function.envelope().code().atom_relocations()).filter_map(|relocation| match relocation.atom() { ImageAtom::Null => None, ImageAtom::Index(value) => Some(value), ImageAtom::Predefined(atom) => Some(atom.raw()), ImageAtom::Dynamic(atom) => Some(atom.zero_based()), }).collect() } }'
expect_rejected bytecode-image-type-alias bytecode-image-alias \
    src/engine/code/binary_object/bytecode_image/model.rs \
    'type BytecodeImageAlias = BytecodeImage;'
expect_rewrite_rejected bytecode-image-raw-u32-method bytecode-image-visible-method-set \
    src/engine/code/binary_object/bytecode_image/model.rs \
    $'impl BytecodeImage {\n    fn sab_archive_occurrences(&self) {}' \
    $'impl BytecodeImage {\n    pub(in crate::engine::code) fn leaked_raw_atom(&self, atom: ImageAtom) -> Option<u32> { match atom { ImageAtom::Null | ImageAtom::Dynamic(_) => None, ImageAtom::Index(raw) => Some(raw), ImageAtom::Predefined(atom) => Some(atom.raw()), } }\n    fn sab_archive_occurrences(&self) {}'
expect_full_rewrite_rejected bytecode-image-helper-indirection bytecode-image-model-seal \
    src/engine/code/binary_object/bytecode_image/model.rs \
    $'    pub(in crate::engine::code) const fn operand_offset(self) -> u32 {\n        self.operand_offset\n    }' \
    $'    pub(in crate::engine::code) const fn operand_offset(self) -> u32 {\n        Self::leak_atom_identity(self.atom)\n    }\n\n    const fn leak_atom_identity(atom: ImageAtom) -> u32 {\n        match atom {\n            ImageAtom::Null => 0,\n            ImageAtom::Index(value) => value,\n            ImageAtom::Predefined(atom) => atom.raw(),\n            ImageAtom::Dynamic(atom) => atom.zero_based(),\n        }\n    }'
expect_rewrite_rejected pinned-eval-identity-drift scalar-script-atom-predicate \
    src/engine/code/binary_object/bytecode_image/model.rs \
    'const PINNED_EVAL_ATOM_RAW: u32 = 84;' \
    'const PINNED_EVAL_ATOM_RAW: u32 = 85;'
