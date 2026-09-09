expect_full_rewrite_table < "$boundary_dir/canaries/stage3g_canaries.txt"
expect_full_rewrite_rejected stage3g-translate-object-erased \
    stage3g-object-translation-route \
    src/engine/code/binary_object/function_translate/mod.rs \
    '                PendingOperation::Ready(operation) => operation,' \
    $'                PendingOperation::Ready(FunctionOp::Object) => continue,\n                PendingOperation::Ready(operation) => operation,'
expect_full_rewrite_rejected stage3g-translate-object-alias-erased \
    stage3g-object-translation-route \
    src/engine/code/binary_object/function_translate/mod.rs \
    '                PendingOperation::Ready(operation) => operation,' \
    $'                PendingOperation::Ready(operation) => {\n                    use FunctionOp as O;\n                    if matches!(operation, O::Object) {\n                        continue;\n                    }\n                    operation\n                },'
expect_full_rewrite_rejected stage3g-object-verifier-terminal-bypass \
    stage3g-object-verifier src/engine/code/bytecode.rs \
    $'            Instruction::ThrowReadOnly(_)\n            | Instruction::ThrowRedeclaration(_)' \
    $'            Instruction::Object\n            | Instruction::ThrowReadOnly(_)\n            | Instruction::ThrowRedeclaration(_)'
expect_full_rewrite_rejected stage3g-runtime-test-macro-shadow \
    stage3e-runtime-evidence src/engine/heap/runtime/tests.rs \
    $'fn trusted_quickjs_ordinary_object_verification_rolls_back_and_retries() {\n    let mut object_only = QUICKJS_ORDINARY_OBJECT_BC5.to_vec();' \
    $'fn trusted_quickjs_ordinary_object_verification_rolls_back_and_retries() {\n    macro_rules! assert_eq { ($($tokens:tt)*) => {}; }\n    let mut object_only = QUICKJS_ORDINARY_OBJECT_BC5.to_vec();'
expect_full_rewrite_rejected stage3g-status-typed-hidden-wrapper \
    stage3g-status docs/status.md \
    'Stage 3G admits raw 11 only as the exact one-to-one typed chain' \
    $'<div hidden>\nStage 3G admits raw 11 only as the exact one-to-one typed chain' \
    $'Stage 3G exposes no new source syntax, public API, Test262\nadmission, or Feature Parity claim.' \
    $'Stage 3G exposes no new source syntax, public API, Test262\nadmission, or Feature Parity claim.\n</div>'
expect_full_rewrite_rejected stage3g-status-inherited-lifecycle-comment-wrapper \
    stage3g-status docs/status.md \
    $'Stage-3G\nraw-11 Object' \
    $'<!--\nStage-3G\nraw-11 Object' \
    'raw-177 coverage, and makes no new conformance claim.' \
    $'raw-177 coverage, and makes no new conformance claim.\n-->'
