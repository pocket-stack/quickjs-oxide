expect_rewrite_rejected ordinary-consumer-float-normalization ordinary-leaf-consumer-lowering \
    src/engine/code/binary_object_publish.rs \
    'DetachedPrimitive::Float64Bits(bits) => Value::Float(f64::from_bits(bits)),' \
    'DetachedPrimitive::Float64Bits(bits) => Value::number(f64::from_bits(bits)),'
expect_rewrite_rejected ordinary-consumer-op-remap ordinary-leaf-consumer-lowering \
    src/engine/code/binary_object_publish.rs \
    'OrdinaryLeafBinaryOp::Add => Instruction::Add,' \
    'OrdinaryLeafBinaryOp::Add => Instruction::Sub,'
expect_rewrite_rejected ordinary-consumer-verifier-dead-branch ordinary-leaf-consumer-publication \
    src/engine/code/binary_object_publish.rs \
    $'        let function = super::bytecode_publish::VerifiedFunction::ordinary_leaf(function)\n            .map_err(map_ordinary_leaf_verification_error)?;' \
    $'        if false {\n            let function = super::bytecode_publish::VerifiedFunction::ordinary_leaf(function)\n                .map_err(map_ordinary_leaf_verification_error)?;\n        }'
expect_rewrite_rejected ordinary-consumer-generic-publisher ordinary-leaf-consumer-publication \
    src/engine/code/binary_object_publish.rs \
    'self.publish_verified_unlinked_function(realm, function)?' \
    'self.publish_unlinked_function(realm, function)?'
expect_full_rewrite_rejected ordinary-consumer-raw-native-plan ordinary-leaf-consumer-import \
    src/engine/code/binary_object_publish.rs \
    $'#[cfg(test)]\nmod tests {' \
    $'fn leak_native_plan(_: NativeCodePlan<'"'"'_>) {}\n\n#[cfg(test)]\nmod tests {'
expect_full_rewrite_rejected ordinary-consumer-test262-branch ordinary-leaf-consumer-special-casing \
    src/engine/code/binary_object_publish.rs \
    $'#[cfg(test)]\nmod tests {' \
    $'fn fixture_dispatch(bytes: &[u8]) -> bool { bytes.starts_with(&[0x05, 0x00]) } // Test262 fixture\n\n#[cfg(test)]\nmod tests {'
expect_rejected root-public-module root-module-visibility \
    src/engine/code/binary_object/mod.rs \
    'pub(in crate::engine::code) mod leaked;'
expect_rewrite_rejected ordinary-leaf-public-module root-module-visibility \
    src/engine/code/binary_object/mod.rs \
    'mod ordinary_leaf;' \
    'pub(super) mod ordinary_leaf;'
expect_rewrite_rejected scalar-script-public-module root-module-visibility \
    src/engine/code/binary_object/mod.rs \
    'mod scalar_script;' \
    'pub(super) mod scalar_script;'
expect_rejected root-extra-private-module root-private-module-set \
    src/engine/code/binary_object/mod.rs \
    'mod unreviewed_admission;'
expect_rejected root-reexport root-reexport \
    src/engine/code/binary_object/mod.rs \
    'pub(in crate::engine::code) use bytecode_image::*;'
expect_rewrite_rejected scalar-facade-extra-type scalar-script-facade-shape \
    src/engine/code/binary_object/mod.rs \
    'pub(super) use scalar_script::{ScalarScriptReadError, ScalarStringDraft, ScalarUnaryOp, ScalarValueDraft, decode_trusted_scalar_script};' \
    'pub(super) use scalar_script::{BytecodeImage, ScalarScriptReadError, ScalarStringDraft, ScalarUnaryOp, ScalarValueDraft, decode_trusted_scalar_script};'
expect_rewrite_rejected scalar-facade-wider-visibility scalar-script-facade-shape \
    src/engine/code/binary_object/mod.rs \
    'pub(super) use scalar_script::{ScalarScriptReadError, ScalarStringDraft, ScalarUnaryOp, ScalarValueDraft, decode_trusted_scalar_script};' \
    'pub(crate) use scalar_script::{ScalarScriptReadError, ScalarStringDraft, ScalarUnaryOp, ScalarValueDraft, decode_trusted_scalar_script};'
expect_rewrite_rejected ordinary-facade-extra-type ordinary-leaf-facade-shape \
    src/engine/code/binary_object/mod.rs \
    'DetachedPrimitive, OrdinaryLeafApplyKind, OrdinaryLeafBinaryOp, OrdinaryLeafDraft,' \
    'BytecodeImage, DetachedPrimitive, OrdinaryLeafApplyKind, OrdinaryLeafBinaryOp, OrdinaryLeafDraft,'
expect_rewrite_rejected ordinary-facade-wider-visibility ordinary-leaf-facade-shape \
    src/engine/code/binary_object/mod.rs \
    'pub(super) use ordinary_leaf::{' \
    'pub(crate) use ordinary_leaf::{'
expect_rejected ordinary-extra-visible-item ordinary-leaf-visible-item-set \
    src/engine/code/binary_object/ordinary_leaf.rs \
    'pub(in crate::engine::code) fn leak_archive_identity() {}'
expect_rejected ordinary-private-helper ordinary-leaf-helper-set \
    src/engine/code/binary_object/ordinary_leaf.rs \
    'fn bypass_ordinary_admission() {}'
expect_rejected ordinary-raw-code-dependency ordinary-leaf-native-plan-boundary \
    src/engine/code/binary_object/ordinary_leaf.rs \
    'fn leak_raw_code(_: &ImageCode) {}'
expect_rewrite_rejected ordinary-input-prefix-dispatch ordinary-leaf-special-casing \
    src/engine/code/binary_object/ordinary_leaf.rs \
    '    if input.len() > MAX_INPUT_BYTES {' \
    '    if input.starts_with(&[0x05, 0x00]) || input.len() > MAX_INPUT_BYTES {'
expect_rewrite_rejected ordinary-verifier-strip-bypass ordinary-leaf-verifier-role \
    src/engine/code/verify/roles.rs \
    '                    || !metadata.strip_variable_debug' \
    '                        || false'
expect_rewrite_rejected ordinary-verifier-debug-bypass ordinary-leaf-verifier-role \
    src/engine/code/verify/roles.rs \
    '                    || function.debug().is_some()' \
    '                        || false'
expect_rewrite_rejected ordinary-verifier-primitive-broadening ordinary-leaf-plain-primitive \
    src/engine/code/function.rs \
    'matches!(self.0, UnlinkedConstantKind::Primitive(_))' \
    'matches!(self.0, UnlinkedConstantKind::Primitive(_) | UnlinkedConstantKind::AtomString(_))'
expect_rewrite_rejected ordinary-verifier-empty-atom-broadening ordinary-leaf-plain-primitive \
    src/engine/code/function.rs \
    'UnlinkedConstantKind::AtomString(PrimitiveValue::String(value)) if value.is_empty()' \
    'UnlinkedConstantKind::AtomString(PrimitiveValue::String(_))'
expect_rewrite_rejected ordinary-public-api-selector-collapse ordinary-leaf-public-api \
    src/engine/api/context/bytecode.rs \
    $'            bytes,\n            root_constant_index,\n        );' \
    $'            bytes,\n            0,\n        );'
expect_rewrite_rejected ordinary-public-api-pending-broadening ordinary-leaf-public-api \
    src/engine/api/context/bytecode.rs \
    $'            Ok(function) => Ok(function),\n            Err(RuntimeError::Engine(error))\n                if NativeErrorKind::from_javascript_error(error.kind()).is_some() =>' \
    $'            Ok(function) => Ok(function),\n            Err(RuntimeError::Engine(error))\n                if true || NativeErrorKind::from_javascript_error(error.kind()).is_some() =>'
expect_rewrite_rejected scalar-draft-copy-regression scalar-script-draft-shape \
    src/engine/code/binary_object/scalar_script.rs \
    $'#[derive(Clone, Debug, Eq, PartialEq)]\npub(in crate::engine::code) enum ScalarValueDraft' \
    $'#[derive(Clone, Copy, Debug, Eq, PartialEq)]\npub(in crate::engine::code) enum ScalarValueDraft'
expect_rewrite_rejected scalar-unary-chain-storage scalar-script-sequence-shape \
    src/engine/code/binary_object/scalar_script.rs \
    '    unary_ops: Box<[ScalarUnaryOp]>,' \
    '    unary_ops: Vec<ScalarUnaryOp>,'
expect_rejected scalar-opcode-set-widening scalar-script-opcode-set \
    src/engine/code/binary_object/scalar_script.rs \
    'const OP_PUSH_THIS: u8 = 0x08;'
expect_rewrite_rejected scalar-unary-name-widening scalar-unary-operation-shape \
    src/engine/code/binary_object/scalar_script.rs \
    '            FunctionUnaryOp::TypeOf => Self::TypeOf,' \
    $'            FunctionUnaryOp::TypeOf => Self::TypeOf,\n            FunctionUnaryOp::Neg => Self::TypeOf,'

expect_full_rewrite_rejected published-function-skip-verifier published-function-verification \
    src/engine/code/verify/verified.rs \
    '        verify_unlinked_ordinary_leaf(&function)?;' \
    '        // skipped by mutation'
expect_full_rewrite_rejected published-function-public-draft published-function-verification \
    src/engine/code/verify/verified.rs \
    'pub(crate) struct VerifiedFunction(UnlinkedFunction);' \
    'pub(crate) struct VerifiedFunction(pub(crate) UnlinkedFunction);'

if grep -q 'fn snapshot_function_bytecode_owned' "$repository_root/src/engine/code/executable.rs"; then
    expect_full_rewrite_rejected published-executable-wrong-runtime published-executable-owner \
        src/engine/code/executable.rs \
        '        self.snapshot_function_bytecode_owned(function.clone())' \
        '        self.unchecked_snapshot(function.clone())'
else
    expect_full_rewrite_rejected published-executable-wrong-runtime published-executable-owner \
        src/engine/code/executable.rs \
        '        if !function.belongs_to(self) {' \
        '        if false {'
fi
if grep -q 'data: Rc<PublishedFunctionData>' "$repository_root/src/engine/code/executable.rs"; then
    publication_data_type='Rc<PublishedFunctionData>'
else
    publication_data_type='PublishedFunctionData'
fi
expect_full_rewrite_rejected published-executable-mutable-layout published-executable-owner \
    src/engine/code/executable.rs \
    "    data: $publication_data_type," \
    "    pub(crate) data: $publication_data_type,"

expect_full_rewrite_rejected published-frame-code-substitution published-frame-owner \
    src/engine/vm/host_bridge.rs \
    '        Ok((self.executable.code.clone(), activation))' \
    '        Ok((Rc::from([]), activation))'

expect_full_rewrite_rejected published-branch-general-check published-static-target \
    src/engine/vm/protocol.rs \
    '        super::activation::checked_target(target, code_len)' \
    '        Ok(target as usize)'
expect_full_rewrite_rejected published-branch-fixture-check published-static-target \
    src/engine/vm/host_bridge.rs \
    '            return super::activation::checked_target(target, _code_len);' \
    '            return Ok(target as usize);'

expect_rewrite_rejected ordinary-verifier-role-call-bypass ordinary-leaf-verifier-dispatch \
    src/engine/code/verify/mod.rs \
    '        roles::verify(function, is_root, root_publication)?;' \
    '        if false { roles::verify(function, is_root, root_publication)?; }'

expect_rewrite_rejected ordinary-verifier-test-gate-bypass ordinary-leaf-verifier-test-gate \
    src/engine/code/verify/mod.rs \
    $'#[cfg(test)]\nmod tests;' \
    'mod tests;'

expect_rewrite_rejected frame-layout-operand-capacity-bypass published-frame-layout \
    src/engine/code/function/layout.rs \
    'usize::from(self.metadata.max_stack)' \
    '0'
expect_rewrite_rejected frame-layout-actual-arguments-truncation published-frame-layout \
    src/engine/code/function/layout.rs \
    'actual_count.max(usize::from(self.metadata.argument_count))' \
    'usize::from(self.metadata.argument_count)'

expect_full_rewrite_rejected published-cache-drop-root published-executable-owner \
    src/engine/code/executable.rs \
    '            root: std::cell::OnceCell::from(function),' \
    '            root: Default::default(),'
expect_full_rewrite_rejected published-cache-wrong-node published-executable-owner \
    src/engine/code/executable.rs \
    'let bytecode = state.heap.function_bytecode(function.bytecode_id())?;' \
    'let bytecode = state.heap.function_bytecode(other.bytecode_id())?;'
expect_full_rewrite_rejected published-cache-skip-realm published-executable-owner \
    src/engine/code/executable.rs \
    '        state.heap.context(bytecode.realm)?;' \
    '        // missing realm authentication'
expect_full_rewrite_rejected published-cache-eval-index-alias published-executable-owner \
    src/engine/code/executable.rs \
    'self.index == other.index && Rc::ptr_eq(&self.environments, &other.environments)' \
    'Rc::ptr_eq(&self.environments, &other.environments)'
expect_full_rewrite_rejected published-function-timed-dead-verifier published-function-verification \
    src/engine/code/verify/verified.rs \
    '        verify_unlinked_tree(&function)?;' \
    '        if false { verify_unlinked_tree(&function)?; }'
