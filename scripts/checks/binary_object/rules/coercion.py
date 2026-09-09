"""Coercion checks, in the ordered boundary scan."""
from __future__ import annotations

import hashlib
import re
from copy import deepcopy

from ..evidence import coercion as evidence


def check(ctx):
    if ctx.self_test_marker_authorized:
        return

    stage3j_rust_diff_sha256 = (
        "ac1919d895e07cbbdf4134e14c8c3a5318734a9578216ac65c949464508d33b5"
    )

    stage3j_rust_file_hashes = deepcopy(evidence.STAGE3J_RUST_FILE_HASHES)

    for ctx.relative, ctx.expected_hash in stage3j_rust_file_hashes.items():
        ctx.path = ctx.root / ctx.relative
        if ctx.path.is_symlink() or not ctx.path.is_file():
            ctx.fail("stage3j-rust-freeze", f"{ctx.relative} must remain a regular frozen Stage3J Rust file")
            continue
        ctx.found_hash = hashlib.sha256(ctx.path.read_bytes()).hexdigest()
        if ctx.found_hash != ctx.expected_hash:
            ctx.fail(
                "stage3j-rust-freeze",
                f"{ctx.relative} drifted from the reviewed test-layout snapshot (original Rust6 diff {stage3j_rust_diff_sha256}); found {ctx.found_hash}",
            )

    ctx.require_normalized_code_sha256(
        "stage3g-object-translation-route",
        "raw11 Object lowering must remain an operand-free one-operation translation with no alias or expansion",
        ctx.stage3b_function(
            "src/engine/code/binary_object/function_translate/mod.rs",
            "lower_operation",
            "stage3g-object-translation-route",
        ),
        "80936a383d38195b24c64242e118ef35944e4a22286da867eeddffe438327ba8",
    )

    ctx.require_normalized_code_sha256(
        "stage3g-object-translation-route",
        "translate_native_plan must retain its exact alias-free two-pass publisher so raw11 Object cannot be intercepted, erased, remapped, or index-collapsed before publication",
        ctx.translate_native_item,
        "fe149677e125ffef44ebac61b8b9799eff3166eafd7efa93e086b84571eb9867",
    )

    if re.search(
        r"\b(?:Recipe|FunctionOp|OrdinaryLeafOp|Instruction)"
        r"[ \t\n]*::[ \t\n]*Object\b",
        ctx.translate_native_item,
    ):
        ctx.fail(
            "stage3g-object-translation-route",
            "raw11 Object must not acquire a pre-match, second-pass dispatch, erase, remap, or index-collapse path outside the typed lower_operation handoff",
        )

    ctx.require_normalized_code_sha256(
        "stage3g-object-ordinary-route",
        "FunctionOp::Object must reach exactly one OrdinaryLeafOp::Object without erasure or remap",
        ctx.stage3b_function(
            "src/engine/code/binary_object/ordinary_leaf.rs",
            "lower_operation",
            "stage3g-object-ordinary-route",
        ),
        "32c17de1021480b9ac7eaf63113a9575098b4329c6f4261eec12a65b6498816a",
    )

    ctx.require_normalized_code_sha256(
        "stage3g-object-publication",
        "OrdinaryLeafOp::Object must publish exactly Instruction::Object without dropping it or touching the synthetic-constant index",
        ctx.stage3b_function(
            "src/engine/code/binary_object_publish.rs",
            "lower_ordinary_leaf_op",
            "stage3g-object-publication",
        ),
        "0fe1f20d09e3441228acc24c6f93c88693c858afe5f7b8d61662eaa99bc14aaa",
    )

    ctx.require_normalized_code_sha256(
        "stage3h-to-object-translation-route",
        "raw111 ToObject lowering must remain an operand-free one-operation translation with no alias or expansion",
        ctx.stage3b_function(
            "src/engine/code/binary_object/function_translate/mod.rs",
            "lower_operation",
            "stage3h-to-object-translation-route",
        ),
        "80936a383d38195b24c64242e118ef35944e4a22286da867eeddffe438327ba8",
    )

    ctx.require_normalized_code_sha256(
        "stage3h-to-object-translation-route",
        "translate_native_plan must retain its exact alias-free two-pass publisher so raw111 ToObject cannot be intercepted, erased, remapped, or index-collapsed before publication",
        ctx.translate_native_item,
        "fe149677e125ffef44ebac61b8b9799eff3166eafd7efa93e086b84571eb9867",
    )

    if re.search(
        r"\b(?:Recipe|FunctionOp|OrdinaryLeafOp|Instruction)"
        r"[ \t\n]*::[ \t\n]*ToObject\b",
        ctx.translate_native_item,
    ):
        ctx.fail(
            "stage3h-to-object-translation-route",
            "raw111 ToObject must not acquire a pre-match, second-pass dispatch, erase, remap, or index-collapse path outside the typed lower_operation handoff",
        )

    ctx.require_normalized_code_sha256(
        "stage3h-to-object-ordinary-route",
        "FunctionOp::ToObject must reach exactly one OrdinaryLeafOp::ToObject without erasure or remap",
        ctx.stage3b_function(
            "src/engine/code/binary_object/ordinary_leaf.rs",
            "lower_operation",
            "stage3h-to-object-ordinary-route",
        ),
        "32c17de1021480b9ac7eaf63113a9575098b4329c6f4261eec12a65b6498816a",
    )

    ctx.require_normalized_code_sha256(
        "stage3h-to-object-publication",
        "OrdinaryLeafOp::ToObject must publish exactly Instruction::ToObject without dropping it or touching the synthetic-constant index",
        ctx.stage3b_function(
            "src/engine/code/binary_object_publish.rs",
            "lower_ordinary_leaf_op",
            "stage3h-to-object-publication",
        ),
        "0fe1f20d09e3441228acc24c6f93c88693c858afe5f7b8d61662eaa99bc14aaa",
    )

    ctx.require_normalized_code_sha256(
        "stage3i-push-this-translation-route",
        "raw8 PushThis lowering must remain an operand-free one-operation translation with no recipe or DTO alias",
        ctx.stage3b_function(
            "src/engine/code/binary_object/function_translate/mod.rs",
            "lower_operation",
            "stage3i-push-this-translation-route",
        ),
        "80936a383d38195b24c64242e118ef35944e4a22286da867eeddffe438327ba8",
    )

    ctx.require_normalized_code_sha256(
        "stage3i-push-this-translation-route",
        "translate_native_plan must retain its exact alias-free source-to-output map so raw8 cannot be intercepted, erased, expanded, remapped, or index-collapsed",
        ctx.translate_native_item,
        "fe149677e125ffef44ebac61b8b9799eff3166eafd7efa93e086b84571eb9867",
    )

    if re.search(
        r"\b(?:Recipe|FunctionOp|OrdinaryLeafOp|Instruction)"
        r"[ \t\n]*::[ \t\n]*PushThis\b",
        ctx.translate_native_item,
    ):
        ctx.fail(
            "stage3i-push-this-translation-route",
            "raw8 PushThis must not acquire a pre-match, second-pass dispatch, erase, remap, or source/output index-collapse path outside the typed lower_operation handoff",
        )

    stage3i_lower_code = ctx.stage3b_function(
        "src/engine/code/binary_object/ordinary_leaf.rs",
        "lower_code",
        "stage3i-push-this-protocol",
    )

    ctx.require_normalized_code_sha256(
        "stage3i-push-this-protocol",
        "ordinary lowering must validate the raw8 entrance protocol before atom accounting, operation lowering, or publication",
        stage3i_lower_code,
        "802efb137202fe4c4b7380359e627b9e97d676ed0e19b6041a58230e03820557",
    )

    ctx.require_ordered_fragments(
        "stage3i-push-this-protocol",
        "validate_push_this_protocol(code) must run exactly once before the input-atom ledger and typed output loop",
        stage3i_lower_code,
        (
            "validate_push_this_protocol(code)?;",
            "let mut input_atoms = InputAtomLedger::new(input_atom_slot_count)?;",
            "for instruction in code.instructions() {",
        ),
    )

    stage3i_validator = ctx.stage3b_function(
        "src/engine/code/binary_object/ordinary_leaf.rs",
        "validate_push_this_protocol",
        "stage3i-push-this-protocol",
    )

    ctx.require_normalized_code_sha256(
        "stage3i-push-this-protocol",
        "the raw8 entrance validator must preserve its exact count, typed-index-zero, and explicit-branch-target-zero predicates",
        stage3i_validator,
        "be9784b4c3c11d623428f5be036b0c01445dfc61a7bf32b5ad8d515762fe451d",
    )

    normalized_stage3i_validator = " ".join(stage3i_validator.split())

    stage3i_validator_fragments = deepcopy(evidence.STAGE3I_VALIDATOR_FRAGMENTS)

    if any(
        normalized_stage3i_validator.count(fragment) != 1
        for fragment in stage3i_validator_fragments
    ):
        ctx.fail(
            "stage3i-push-this-protocol",
            "raw8-absent bodies must remain unchanged, while a present raw8 must occur exactly once at typed index zero and have no explicit IfFalse, IfTrue, or Goto edge back to zero",
        )

    ctx.require_normalized_code_sha256(
        "stage3i-push-this-ordinary-route",
        "FunctionOp::PushThis must reach exactly one OrdinaryLeafOp::PushThis without a direct, aliased, helper-mediated, or pre-match bypass",
        ctx.stage3b_function(
            "src/engine/code/binary_object/ordinary_leaf.rs",
            "lower_operation",
            "stage3i-push-this-ordinary-route",
        ),
        "32c17de1021480b9ac7eaf63113a9575098b4329c6f4261eec12a65b6498816a",
    )

    ctx.require_normalized_code_sha256(
        "stage3i-push-this-publication",
        "OrdinaryLeafOp::PushThis must publish exactly Instruction::PushThis without dropping it or consuming a synthetic constant index",
        ctx.stage3b_function(
            "src/engine/code/binary_object_publish.rs",
            "lower_ordinary_leaf_op",
            "stage3i-push-this-publication",
        ),
        "0fe1f20d09e3441228acc24c6f93c88693c858afe5f7b8d61662eaa99bc14aaa",
    )

    stage3j_translate_lower = ctx.stage3b_function(
        "src/engine/code/binary_object/function_translate/mod.rs",
        "lower_operation",
        "stage3j-to-propkey-translation-route",
    )

    ctx.require_normalized_code_sha256(
        "stage3j-to-propkey-translation-route",
        "raw112 ToPropKey lowering must remain an operand-free one-operation translation with no recipe or DTO alias",
        stage3j_translate_lower,
        "80936a383d38195b24c64242e118ef35944e4a22286da867eeddffe438327ba8",
    )

    ctx.require_normalized_code_sha256(
        "stage3j-to-propkey-translation-route",
        "translate_native_plan must retain its exact alias-free source-to-output map so raw112 cannot be intercepted, erased, expanded, remapped, or index-collapsed",
        ctx.translate_native_item,
        "fe149677e125ffef44ebac61b8b9799eff3166eafd7efa93e086b84571eb9867",
    )

    if re.search(
        r"\b(?:Recipe|FunctionOp|OrdinaryLeafOp|Instruction)"
        r"[ \t\n]*::[ \t\n]*ToPropKey\b",
        ctx.translate_native_item,
    ):
        ctx.fail(
            "stage3j-to-propkey-translation-route",
            "raw112 ToPropKey must not acquire a pre-match, second-pass dispatch, erase, remap, expansion, or source/output index-collapse path outside the typed lower_operation handoff",
        )

    stage3j_ordinary_lower = ctx.stage3b_function(
        "src/engine/code/binary_object/ordinary_leaf.rs",
        "lower_operation",
        "stage3j-to-propkey-ordinary-route",
    )

    ctx.require_normalized_code_sha256(
        "stage3j-to-propkey-ordinary-route",
        "FunctionOp::ToPropKey must reach exactly one OrdinaryLeafOp::ToPropKey without erasure, remap, metadata mutation, or stack rewriting",
        stage3j_ordinary_lower,
        "32c17de1021480b9ac7eaf63113a9575098b4329c6f4261eec12a65b6498816a",
    )

    stage3j_publisher = ctx.stage3b_function(
        "src/engine/code/binary_object_publish.rs",
        "lower_ordinary_leaf_op",
        "stage3j-to-propkey-publication",
    )

    ctx.require_normalized_code_sha256(
        "stage3j-to-propkey-publication",
        "OrdinaryLeafOp::ToPropKey must publish exactly Instruction::ToPropKey without synthetic constants, metadata mutation, or stack rewriting",
        stage3j_publisher,
        "0fe1f20d09e3441228acc24c6f93c88693c858afe5f7b8d61662eaa99bc14aaa",
    )

    normalized_stage3j_translate = " ".join(stage3j_translate_lower.split())

    normalized_stage3j_ordinary = " ".join(stage3j_ordinary_lower.split())

    normalized_stage3j_publisher = " ".join(stage3j_publisher.split())

    if (
        normalized_stage3j_translate.count(
            "(Recipe::ToPropKey, NativeOperands::None) => ready(FunctionOp::ToPropKey),"
        ) != 1
        or normalized_stage3j_ordinary.count(
            "FunctionOp::ToPropKey => Ok(OrdinaryLeafOp::ToPropKey),"
        ) != 1
        or normalized_stage3j_publisher.count(
            "OrdinaryLeafOp::ToPropKey => Instruction::ToPropKey,"
        ) != 1
    ):
        ctx.fail(
            "stage3j-to-propkey-typed-chain",
            "raw112 must retain the unique exact Recipe::ToPropKey to FunctionOp::ToPropKey to OrdinaryLeafOp::ToPropKey to Instruction::ToPropKey chain",
        )

    if (
        "ToPropKey" in stage3i_lower_code
        or "ToPropKey" in stage3i_validator
        or re.search(r"ToPropKey.{0,160}next_synthetic_index", normalized_stage3j_publisher)
        or re.search(r"next_synthetic_index.{0,160}ToPropKey", normalized_stage3j_publisher)
    ):
        ctx.fail(
            "stage3j-to-propkey-protocol",
            "raw112 must use the existing ordinary 1-to-1 stack/CFG verifier and must not acquire raw8 exact-one/index-zero/no-target narrowing or synthetic-constant accounting",
        )

    stage3j_host_to_propkey = ctx.stage3b_function(
        "src/engine/vm/host_bridge.rs",
        "convert_property_key",
        "stage3j-to-propkey-host-semantics",
    )

    stage3j_host_to_propkey_source = ctx.stage3j_source_function(
        "src/engine/vm/host_bridge.rs",
        "convert_property_key",
        "stage3j-to-propkey-host-semantics",
    )

    ctx.require_normalized_code_sha256(
        "stage3j-to-propkey-host-semantics",
        "RuntimeVmHost::convert_property_key must retain its exact primitive and Symbol identity fast paths, string-hint Object conversion, thrown-value propagation, and canonical String fallback",
        stage3j_host_to_propkey,
        "b3f51a60bdc4fb1816ecc18a05cdb58679616bb3ebeee787d8767bc0156f3d39",
    )

    ctx.require_normalized_code_sha256(
        "stage3j-to-propkey-host-semantics",
        "the bounded raw RuntimeVmHost::convert_property_key source, including semantic literals, must remain exact",
        stage3j_host_to_propkey_source,
        "8a2c3c601d0a2243e99a8116603133c3f355527c0010b0023e4addff17f0618d",
    )

    normalized_stage3j_host_to_propkey = " ".join(
        stage3j_host_to_propkey_source.split()
    )

    stage3j_host_to_propkey_fragments = deepcopy(evidence.STAGE3J_HOST_TO_PROPKEY_FRAGMENTS)

    if any(
        normalized_stage3j_host_to_propkey.count(fragment) != expected_count
        for fragment, expected_count in stage3j_host_to_propkey_fragments
    ):
        ctx.fail(
            "stage3j-to-propkey-host-semantics",
            "ToPropKey host conversion must preserve Int/String and Symbol identity, runtime ownership, the defining-realm string hint, arbitrary Throw identity, and canonical primitive-to-String fallback",
        )

    stage3j_to_primitive = ctx.stage3b_function(
        "src/engine/heap/runtime/mod.rs",
        "to_primitive",
        "stage3j-to-propkey-primitive-semantics",
    )

    stage3j_to_primitive_source = ctx.stage3j_source_function(
        "src/engine/heap/runtime/mod.rs",
        "to_primitive",
        "stage3j-to-propkey-primitive-semantics",
    )

    ctx.require_normalized_code_sha256(
        "stage3j-to-propkey-primitive-semantics",
        "Runtime::to_primitive must retain primitive identity, @@toPrimitive lookup/call/Throw behavior, defining-realm TypeErrors, and ordinary fallback",
        stage3j_to_primitive,
        "abcd2b0699c336532fa0b0f19ba13a36a912b9801ebaacbff6b497ac2becb727",
    )

    ctx.require_normalized_code_sha256(
        "stage3j-to-propkey-primitive-semantics",
        "the bounded raw Runtime::to_primitive source, including the exact hint and error literals, must remain exact",
        stage3j_to_primitive_source,
        "fe6d2c76335f23517797e73f3341a176328dcde4aca78a2cbbc9be22f36d5592",
    )

    normalized_stage3j_to_primitive = " ".join(stage3j_to_primitive_source.split())

    stage3j_to_primitive_fragments = deepcopy(evidence.STAGE3J_TO_PRIMITIVE_FRAGMENTS)

    if any(
        normalized_stage3j_to_primitive.count(fragment) != expected_count
        for fragment, expected_count in stage3j_to_primitive_fragments
    ):
        ctx.fail(
            "stage3j-to-propkey-primitive-semantics",
            "@@toPrimitive must receive the exact string hint once, preserve primitive and Throw identity, reject Object results with a defining-realm TypeError, and fall back ordinarily",
        )

    stage3j_ordinary_to_primitive = ctx.stage3b_function(
        "src/engine/builtins/object.rs",
        "ordinary_to_primitive",
        "stage3j-to-propkey-ordinary-fallback",
    )

    stage3j_ordinary_to_primitive_source = ctx.stage3j_source_function(
        "src/engine/builtins/object.rs",
        "ordinary_to_primitive",
        "stage3j-to-propkey-ordinary-fallback",
    )

    ctx.require_normalized_code_sha256(
        "stage3j-to-propkey-ordinary-fallback",
        "Runtime::ordinary_to_primitive must retain callable lookup, completion propagation, Object-result retry, and defining-realm failure",
        stage3j_ordinary_to_primitive,
        "f19aa3f0ecca0598753bc7c8daa96fb546d98b518e17b10c64d1c6f7ea403c7b",
    )

    ctx.require_normalized_code_sha256(
        "stage3j-to-propkey-ordinary-fallback",
        "the bounded raw ordinary ToPrimitive fallback, including exact method ordering, must remain exact",
        stage3j_ordinary_to_primitive_source,
        "85e0451154b83269f31f8c9a85ae64e3c6d39ab31777894bb3b96a57dc1c5aef",
    )

    normalized_stage3j_ordinary_to_primitive = " ".join(
        stage3j_ordinary_to_primitive_source.split()
    )

    stage3j_ordinary_to_primitive_fragments = deepcopy(evidence.STAGE3J_ORDINARY_TO_PRIMITIVE_FRAGMENTS)

    if any(
        normalized_stage3j_ordinary_to_primitive.count(fragment) != expected_count
        for fragment, expected_count in stage3j_ordinary_to_primitive_fragments
    ):
        ctx.fail(
            "stage3j-to-propkey-ordinary-fallback",
            "ordinary string-hint conversion must call toString before valueOf, preserve getter/call completions, retry only Object results, and materialize the defining-realm TypeError after exhaustion",
        )

    stage3g_test_contracts = deepcopy(evidence.STAGE3G_TEST_CONTRACTS)

    stage3g_test_body_hashes = deepcopy(evidence.STAGE3G_TEST_BODY_HASHES)

    missing_stage3g_tests = []

    drifted_stage3g_tests = []

    for ctx.relative, ctx.name, ctx.anchors in stage3g_test_contracts:
        ctx.code = ctx.stage3b_code(ctx.relative)
        ctx.declarations = list(re.finditer(
            rf"(?P<attributes>(?:#[ \t\n]*\[[^]]*\][ \t\n]*)*)"
            rf"\bfn[ \t\n]+{re.escape(ctx.name)}[ \t\n]*\(",
            ctx.code,
        ))
        if (
            len(ctx.declarations) != 1
            or " ".join(ctx.declarations[0].group("attributes").split()) != "#[test]"
        ):
            missing_stage3g_tests.append(f"{ctx.relative}::{ctx.name}")
            continue
        ctx.declaration_offset = ctx.declarations[0].start()
        ctx.declaration_depth = ctx.code[:ctx.declaration_offset].count("{") - ctx.code[:ctx.declaration_offset].count("}")
        if ctx.relative == "src/engine/heap/runtime/tests.rs":
            ctx.direct_parent = ctx.declaration_depth == 0
        else:
            ctx.parent_bounds = ctx.stage3d_test_parent_bounds.get(ctx.relative)
            ctx.direct_parent = (
                ctx.parent_bounds is not None
                and ctx.parent_bounds[0] < ctx.declaration_offset < ctx.parent_bounds[1]
                and ctx.declaration_depth == 1
            )
        if not ctx.direct_parent:
            missing_stage3g_tests.append(f"{ctx.relative}::{ctx.name} (nested)")
            continue
        ctx.item = ctx.stage3b_function(ctx.relative, ctx.name, "stage3g-runtime-evidence")
        ctx.normalized_item = " ".join(ctx.item.split())
        ctx.item_hash = ctx.normalized_code_sha256(ctx.item)
        if (
            any(anchor not in ctx.normalized_item for anchor in ctx.anchors)
            or ctx.item_hash != stage3g_test_body_hashes.get((ctx.relative, ctx.name))
        ):
            drifted_stage3g_tests.append(f"{ctx.relative}::{ctx.name} ({ctx.item_hash})")

    if missing_stage3g_tests or drifted_stage3g_tests:
        ctx.fail(
            "stage3g-runtime-evidence",
            "Stage3G tests must remain unconditional direct-parent #[test] functions with exact counts/blocker vector, raw11 typed handoff, 41-byte natural wire/max-stack/realm/freshness, transactional rollback/retry, and branch-index evidence; "
            f"missing {missing_stage3g_tests}, drifted {drifted_stage3g_tests}",
        )

    stage3h_test_contracts = deepcopy(evidence.STAGE3H_TEST_CONTRACTS)

    stage3h_test_body_hashes = deepcopy(evidence.STAGE3H_TEST_BODY_HASHES)

    stage3h_test_parent_bounds = dict(ctx.stage3d_test_parent_bounds)

    vm_test_code = ctx.stage3b_code("src/engine/vm/mod.rs")

    vm_test_modules = list(re.finditer(
        r"(?P<attributes>(?:#[ \t\n]*\[[^]]*\][ \t\n]*)*)"
        r"\bmod[ \t\n]+tests[ \t\n]*\{",
        vm_test_code,
    ))

    if (
        len(vm_test_modules) != 1
        or " ".join(vm_test_modules[0].group("attributes").split()) != "#[cfg(test)]"
    ):
        ctx.fail(
            "stage3h-runtime-evidence",
            "src/engine/vm/mod.rs must retain one direct, unconditional #[cfg(test)] tests module",
        )
    else:
        ctx._, vm_module_start, vm_module_end = ctx.braced_item_from_match(
            vm_test_code,
            vm_test_modules[0],
            "stage3h-runtime-evidence",
            "src/engine/vm/mod.rs direct tests module",
        )
        stage3h_test_parent_bounds["src/engine/vm/mod.rs"] = (vm_module_start, vm_module_end)

    stage3h_test_sources = {
        relative for relative, _, _ in stage3h_test_contracts
    }

    for ctx.relative in stage3h_test_sources:
        ctx.code = ctx.stage3b_code(ctx.relative)
        if ctx.assertion_shadow.search(ctx.code):
            ctx.fail(
                "stage3h-runtime-evidence",
                f"{ctx.relative} must not shadow or import the assertion macros used by Stage3H evidence",
            )
        if re.search(
            r"(?m)^[ \t]*#![ \t]*\[[ \t]*(?:cfg|cfg_attr)\b",
            ctx.code,
        ):
            ctx.fail(
                "stage3h-runtime-evidence",
                f"{ctx.relative} must not conditionally exclude its Stage3H test evidence",
            )

    missing_stage3h_tests = []

    drifted_stage3h_tests = []

    for ctx.relative, ctx.name, ctx.anchors in stage3h_test_contracts:
        ctx.code = ctx.stage3b_code(ctx.relative)
        ctx.declarations = list(re.finditer(
            rf"(?P<attributes>(?:#[ \t\n]*\[[^]]*\][ \t\n]*)*)"
            rf"\bfn[ \t\n]+{re.escape(ctx.name)}[ \t\n]*\(",
            ctx.code,
        ))
        if (
            len(ctx.declarations) != 1
            or " ".join(ctx.declarations[0].group("attributes").split()) != "#[test]"
        ):
            missing_stage3h_tests.append(f"{ctx.relative}::{ctx.name}")
            continue
        ctx.declaration_offset = ctx.declarations[0].start()
        ctx.declaration_depth = ctx.code[:ctx.declaration_offset].count("{") - ctx.code[:ctx.declaration_offset].count("}")
        if ctx.relative == "src/engine/heap/runtime/tests.rs":
            ctx.direct_parent = ctx.declaration_depth == 0
        else:
            ctx.parent_bounds = stage3h_test_parent_bounds.get(ctx.relative)
            ctx.direct_parent = (
                ctx.parent_bounds is not None
                and ctx.parent_bounds[0] < ctx.declaration_offset < ctx.parent_bounds[1]
                and ctx.declaration_depth == 1
            )
        if not ctx.direct_parent:
            missing_stage3h_tests.append(f"{ctx.relative}::{ctx.name} (nested)")
            continue
        ctx.item = ctx.stage3b_function(ctx.relative, ctx.name, "stage3h-runtime-evidence")
        ctx.normalized_item = " ".join(ctx.item.split())
        ctx.item_hash = ctx.normalized_code_sha256(ctx.item)
        if (
            any(anchor not in ctx.normalized_item for anchor in ctx.anchors)
            or ctx.item_hash != stage3h_test_body_hashes.get((ctx.relative, ctx.name))
        ):
            drifted_stage3h_tests.append(f"{ctx.relative}::{ctx.name} ({ctx.item_hash})")

    if missing_stage3h_tests or drifted_stage3h_tests:
        ctx.fail(
            "stage3h-runtime-evidence",
            "Stage3H tests must remain unconditional direct-parent #[test] functions with exact counts/blocker vector, raw111 typed handoff, natural/manual wires, object identity, primitive boxing, defining-realm prototypes, nullish pending/catch, rollback/retry, and branch-index evidence; "
            f"missing {missing_stage3h_tests}, drifted {drifted_stage3h_tests}",
        )

    ctx.stage3i_test_contracts = deepcopy(evidence.STAGE3I_TEST_CONTRACTS)
