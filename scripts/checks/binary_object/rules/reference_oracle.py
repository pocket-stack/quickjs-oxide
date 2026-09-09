"""Reference oracle checks, in the ordered boundary scan."""
from __future__ import annotations

import hashlib
from copy import deepcopy

from ..evidence import reference_oracle as evidence


def check(ctx):
    if ctx.self_test_marker_authorized:
        return

    if ctx.missing_stage3j_tests or ctx.drifted_stage3j_tests:
        ctx.fail(
            "stage3j-runtime-evidence",
            "Stage3J tests must remain unconditional direct-parent #[test] functions with exact counts/blockers, raw112 typed chain, natural/manual/duplicate/finite-loop/underflow wires, ordinary verifier acceptance/rejection, no synthetic constants, primitive and Symbol identity, string-hint coercion order, thrown identity, defining-realm TypeError, nested re-entry, strict/sloppy parity, clean pending state, rollback, and retry evidence; "
            f"missing {ctx.missing_stage3j_tests}, drifted {ctx.drifted_stage3j_tests}",
        )

    stage3j_c_diff_sha256 = (
        "707554c4d52518c67226d827e6ce0f2b2b01023b055b8dcc78db4fd9e3e76e72"
    )

    stage3j_c_evidence_hashes = deepcopy(evidence.STAGE3J_C_EVIDENCE_HASHES)

    stage3d_c_sources: dict[str, str] = {}

    for ctx.relative, ctx.expected_hash in stage3j_c_evidence_hashes.items():
        ctx.path = ctx.root / ctx.relative
        if ctx.path.is_symlink() or not ctx.path.is_file():
            ctx.fail("stage3j-c-oracle", f"{ctx.relative} must remain a regular authenticated file")
            continue
        payload = ctx.path.read_bytes()
        ctx.found_hash = hashlib.sha256(payload).hexdigest()
        if ctx.found_hash != ctx.expected_hash:
            ctx.fail(
                "stage3j-c-oracle",
                f"{ctx.relative} drifted from the reviewed fixture-layout snapshot (original C3 diff {stage3j_c_diff_sha256}); found {ctx.found_hash}",
            )
        stage3d_c_sources[ctx.relative] = payload.decode("utf-8")

    stage3d_c_source = stage3d_c_sources.get(
        "apps/cli/tests/fixtures/inputs/function_bytecode_wire.c", ""
    )

    if (
        stage3d_c_source.count(
            "static int expect_ordinary_throw_completion(JSContext *compile_context)"
        ) != 1
        or stage3d_c_source.count(
            "if (expect_ordinary_throw_completion(compile_context))"
        ) != 1
        or stage3d_c_source.count("static const uint8_t ordinary_throw_bytecode[]")
        != 1
        or stage3d_c_source.count(
            "static int expect_ordinary_throw_error_completion(JSContext *compile_context)"
        ) != 1
        or stage3d_c_source.count(
            "if (expect_ordinary_throw_error_completion(compile_context))"
        ) != 1
        or stage3d_c_source.count(
            "static const uint8_t ordinary_throw_error_natural_bytecode[]"
        ) != 1
        or stage3d_c_source.count(
            "static const uint8_t ordinary_throw_error_bytecode[]"
        ) != 1
        or stage3d_c_source.count(
            "static int expect_ordinary_nop_completion(JSContext *compile_context)"
        ) != 1
        or stage3d_c_source.count(
            "if (expect_ordinary_nop_completion(compile_context))"
        ) != 1
        or stage3d_c_source.count(
            "static const uint8_t ordinary_nop_natural_bytecode[]"
        ) != 1
        or stage3d_c_source.count(
            "static const uint8_t ordinary_nop_bytecode[]"
        ) != 1
        or stage3d_c_source.count("memcpy(manual_wire, natural_wire, 39);") != 1
        or stage3d_c_source.count("manual_wire[37] = 2;") != 1
        or stage3d_c_source.count("manual_wire[39] = 177;") != 1
        or stage3d_c_source.count("manual_wire[40] = 41;") != 1
        or stage3d_c_source.count(
            "static int expect_ordinary_object_completion(JSContext *compile_context)"
        ) != 1
        or stage3d_c_source.count(
            "if (expect_ordinary_object_completion(compile_context))"
        ) != 1
        or stage3d_c_source.count(
            "static const uint8_t ordinary_object_bytecode[]"
        ) != 1
        or stage3d_c_source.count(
            '"(function(){\'use strict\';return {};})"'
        ) != 1
        or stage3d_c_source.count(
            "static int expect_ordinary_to_object_completion(JSContext *compile_context)"
        ) != 1
        or stage3d_c_source.count(
            "if (expect_ordinary_to_object_completion(compile_context))"
        ) != 1
        or stage3d_c_source.count(
            "static const uint8_t ordinary_to_object_natural_bytecode[]"
        ) != 1
        or stage3d_c_source.count(
            "static const uint8_t ordinary_to_object_bytecode[]"
        ) != 1
        or stage3d_c_source.count(
            '"(function(a){\'use strict\';({}=a);return a;})"'
        ) != 1
        or stage3d_c_source.count("memcpy(manual_wire, natural_wire, 43);") != 1
        or stage3d_c_source.count("manual_wire[33] = 1;") != 1
        or stage3d_c_source.count("manual_wire[37] = 3;") != 1
        or stage3d_c_source.count("manual_wire[43] = 207;") != 1
        or stage3d_c_source.count("manual_wire[44] = 111;") != 1
        or stage3d_c_source.count("manual_wire[45] = 40;") != 1
    ):
        ctx.fail(
            "stage3d-c-oracle",
            "the authenticated C oracle must define and call exactly one raw48, raw49, raw177, compiler-natural raw11 Object, and raw111 ToObject case, with distinct honest natural/manual wires and exact mechanical derivations",
        )

    if (
        stage3d_c_source.count(
            "static int expect_ordinary_push_this_completion(JSContext *compile_context)"
        ) != 1
        or stage3d_c_source.count(
            "if (expect_ordinary_push_this_completion(compile_context))"
        ) != 1
        or any(
            stage3d_c_source.count(f"static const uint8_t {name}[]") != 1
            for name in (
                "ordinary_push_this_strict_natural_bytecode",
                "ordinary_push_this_sloppy_natural_bytecode",
                "ordinary_push_this_strict_bytecode",
                "ordinary_push_this_sloppy_bytecode",
                "ordinary_push_this_sloppy_duplicate_bytecode",
                "ordinary_push_this_sloppy_loop_bytecode",
            )
        )
        or stage3d_c_source.count("duplicate_raw8_count != 2") != 1
        or stage3d_c_source.count("loop_raw8_count != 1") != 1
        or stage3d_c_source.count("3 + 12 != 15 || 11 - 11 != 0") != 1
    ):
        ctx.fail(
            "stage3i-c-oracle",
            "the authenticated C oracle must define and call one Stage3I raw8 matrix, retain all six exact natural/manual/adversarial wires, and prove duplicate and branch-to-index-zero re-execution",
        )

    if (
        len(stage3d_c_source.splitlines()) != 9981
        or stage3d_c_source.count(
            "static int expect_ordinary_to_propkey_completion("
        ) != 1
        or stage3d_c_source.count(
            "if (expect_ordinary_to_propkey_completion(compile_context))"
        ) != 1
        or any(
            stage3d_c_source.count(f"static const uint8_t {name}[]") != 1
            for name in (
                "ordinary_to_propkey_strict_natural_bytecode",
                "ordinary_to_propkey_sloppy_natural_bytecode",
                "ordinary_to_propkey_loop_base_bytecode",
                "ordinary_to_propkey_strict_bytecode",
                "ordinary_to_propkey_sloppy_bytecode",
                "ordinary_to_propkey_duplicate_bytecode",
                "ordinary_to_propkey_loop_bytecode",
                "ordinary_to_propkey_underflow_bytecode",
            )
        )
        or any(
            stage3d_c_source.count(fragment) != 1
            for fragment in (
                "sizeof(ordinary_to_propkey_strict_natural_bytecode) == 50",
                "sizeof(ordinary_to_propkey_sloppy_natural_bytecode) == 50",
                "sizeof(ordinary_to_propkey_loop_base_bytecode) == 54",
                "sizeof(ordinary_to_propkey_strict_bytecode) == 46",
                "sizeof(ordinary_to_propkey_sloppy_bytecode) == 46",
                "sizeof(ordinary_to_propkey_duplicate_bytecode) == 47",
                "sizeof(ordinary_to_propkey_loop_bytecode) == 65",
                "sizeof(ordinary_to_propkey_underflow_bytecode) == 45",
                "duplicate_raw112_count != 2 || loop_raw112_count != 1",
                "ordinary-to-propkey-underflow-status=",
                "ordinary-to-propkey-oracle=passed",
            )
        )
        or any(
            stage3d_c_source.count(fragment) != 2
            for fragment in (
                "__stage3jNaturalStrict",
                "__stage3jNaturalSloppy",
                "__stage3jStrict",
                "__stage3jSloppy",
                "__stage3jDuplicate",
                "__stage3jLoop",
                "__stage3jDefiningTypeError",
            )
        )
    ):
        ctx.fail(
            "stage3j-c-oracle",
            "the exact 9,981-line C oracle must define and call one Stage3J raw112 matrix with all eight natural/manual/duplicate/finite-loop/underflow wires, defining-realm semantics, repeated execution, and clean pending state",
        )

    stage3d_c_transcript = stage3d_c_sources.get(
        "apps/cli/tests/fixtures/expected/function_bytecode_wire.quickjs-2026-06-04.txt", ""
    )

    stage3d_c_transcript_contract = deepcopy(evidence.STAGE3D_C_TRANSCRIPT_CONTRACT)

    if any(
        stage3d_c_transcript.count(f"{line}\n") != 1
        for line in stage3d_c_transcript_contract
    ):
        ctx.fail(
            "stage3d-c-oracle",
            "the frozen C transcript must retain the exact raw48 wire, metadata, terminal, identity, backtrace, and iterator-close evidence",
        )

    stage3e_c_transcript_contract = deepcopy(evidence.STAGE3E_C_TRANSCRIPT_CONTRACT)

    if any(
        stage3d_c_transcript.count(f"{line}\n") != 1
        for line in stage3e_c_transcript_contract
    ):
        ctx.fail(
            "stage3e-c-oracle",
            "the frozen C transcript must retain both raw49 wire identities, subtype-0 terminal semantics, realm/backtrace/pending/catch evidence, and subtype rejection contrast",
        )

    stage3f_c_transcript_contract = deepcopy(evidence.STAGE3F_C_TRANSCRIPT_CONTRACT)

    if any(
        stage3d_c_transcript.count(f"{line}\n") != 1
        for line in stage3f_c_transcript_contract
    ):
        ctx.fail(
            "stage3f-c-oracle",
            "the frozen C transcript must retain the exact compiler-natural raw41 baseline, mechanically derived raw177/raw41 wire, zero-property metadata, byte identity, repeated cross-realm undefined calls, empty pending state, and C-never-executes malformed boundary",
        )

    stage3g_c_transcript_contract = deepcopy(evidence.STAGE3G_C_TRANSCRIPT_CONTRACT)

    if any(
        stage3d_c_transcript.count(f"{line}\n") != 1
        for line in stage3g_c_transcript_contract
    ):
        ctx.fail(
            "stage3g-c-oracle",
            "the frozen C transcript must retain the exact compiler-natural raw11 Object wire, property-free max-stack-one metadata, read/write identity, fresh defining-realm Objects, and clean pending state",
        )

    stage3h_c_transcript_contract = deepcopy(evidence.STAGE3H_C_TRANSCRIPT_CONTRACT)

    if any(
        stage3d_c_transcript.count(f"{line}\n") != 1
        for line in stage3h_c_transcript_contract
    ):
        ctx.fail(
            "stage3h-c-oracle",
            "the frozen C transcript must retain the natural and mechanical raw111 wire identities, metadata/raw sets, one-to-one stack semantics, object identity, defining-realm primitive boxing, no-coercion, nullish pending/GetException clearing, and caller-catch evidence",
        )

    stage3i_c_transcript_contract = deepcopy(evidence.STAGE3I_C_TRANSCRIPT_CONTRACT)

    if (
        any(
            stage3d_c_transcript.count(f"{line}\n") != 1
            for line in stage3i_c_transcript_contract
        )
    ):
        ctx.fail(
            "stage3i-c-oracle",
            "the authenticated C transcript must retain all strict/sloppy natural/manual raw8 wire identities, metadata and provenance, fresh defining-realm boxing, exact payloads, duplicate and target-zero mismatch probes, pending state, and sole-raw8 admission evidence",
        )

    stage3j_c_transcript_contract = deepcopy(evidence.STAGE3J_C_TRANSCRIPT_CONTRACT)

    if (
        stage3d_c_transcript.count('\n') != 1695
        or any(
            stage3d_c_transcript.count(f"{line}\n") != 1
            for line in stage3j_c_transcript_contract
        )
    ):
        ctx.fail(
            "stage3j-c-oracle",
            "the exact 1,695-line C transcript must retain all strict/sloppy natural/manual raw112 wire identities, duplicate and finite-loop re-entry, underflow non-execution, primitive/Symbol identity, string-hint coercion order, throw/realm/nested semantics, parity, pending cleanup, and sole-raw112 admission evidence",
        )

    stage3i_c_manifest = stage3d_c_sources.get(
        "dev-support/quickjs-c-oracles.tsv", ""
    )

    stage3i_manifest_lines = stage3i_c_manifest.splitlines()

    if (
        len(stage3i_manifest_lines) != 20
        or len(stage3i_manifest_lines[1:]) != 19
        or sum(
            line.startswith(
                "function-bytecode-wire\tfunction-bytecode\t"
                "apps/cli/tests/fixtures/inputs/function_bytecode_wire.c\t"
                "815baf3fbf14de146b53d103401279cd9d5eacd006e60b468f8d141b34e2bd92\t"
                "apps/cli/tests/fixtures/expected/function_bytecode_wire.quickjs-2026-06-04.txt\t"
                "58d8327f176950aeb8ab682dcf8fc11577421c46c5eb146f078adb073fbf03ec\t"
            )
            for line in stage3i_manifest_lines
        )
        != 1
    ):
        ctx.fail(
            "stage3j-c-oracle",
            "the exact 20-line authenticated manifest must retain 19 fixtures and one function-bytecode-wire row pinning the frozen Stage3J C source and transcript hashes",
        )
