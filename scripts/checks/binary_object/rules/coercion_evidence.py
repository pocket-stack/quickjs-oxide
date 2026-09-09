"""Coercion evidence checks, in the ordered boundary scan."""
from __future__ import annotations

import re
from copy import deepcopy

from ..evidence import coercion_evidence as evidence


def check(ctx):
    if ctx.self_test_marker_authorized:
        return

    stage3i_test_body_hashes = deepcopy(evidence.STAGE3I_TEST_BODY_HASHES)

    stage3i_test_parent_bounds = dict(ctx.stage3d_test_parent_bounds)

    stage3i_test_sources = {relative for relative, _, _ in ctx.stage3i_test_contracts}

    for ctx.relative in stage3i_test_sources:
        ctx.code = ctx.stage3b_code(ctx.relative)
        if ctx.assertion_shadow.search(ctx.code):
            ctx.fail(
                "stage3i-runtime-evidence",
                f"{ctx.relative} must not shadow or import the assertion macros used by Stage3I evidence",
            )
        if re.search(r"(?m)^[ \t]*#![ \t]*\[[ \t]*(?:cfg|cfg_attr)\b", ctx.code):
            ctx.fail(
                "stage3i-runtime-evidence",
                f"{ctx.relative} must not conditionally exclude its Stage3I test evidence",
            )

    missing_stage3i_tests = []

    drifted_stage3i_tests = []

    for ctx.relative, ctx.name, ctx.anchors in ctx.stage3i_test_contracts:
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
            missing_stage3i_tests.append(f"{ctx.relative}::{ctx.name}")
            continue
        ctx.declaration_offset = ctx.declarations[0].start()
        ctx.declaration_depth = ctx.code[:ctx.declaration_offset].count("{") - ctx.code[:ctx.declaration_offset].count("}")
        if ctx.relative == "src/engine/heap/runtime/tests.rs":
            ctx.direct_parent = ctx.declaration_depth == 0
        else:
            ctx.parent_bounds = stage3i_test_parent_bounds.get(ctx.relative)
            ctx.direct_parent = (
                ctx.parent_bounds is not None
                and ctx.parent_bounds[0] < ctx.declaration_offset < ctx.parent_bounds[1]
                and ctx.declaration_depth == 1
            )
        if not ctx.direct_parent:
            missing_stage3i_tests.append(f"{ctx.relative}::{ctx.name} (nested)")
            continue
        ctx.item = ctx.stage3b_function(ctx.relative, ctx.name, "stage3i-runtime-evidence")
        ctx.normalized_item = " ".join(ctx.item.split())
        ctx.item_hash = ctx.normalized_code_sha256(ctx.item)
        if (
            any(anchor not in ctx.normalized_item for anchor in ctx.anchors)
            or ctx.item_hash != stage3i_test_body_hashes.get((ctx.relative, ctx.name))
        ):
            drifted_stage3i_tests.append(f"{ctx.relative}::{ctx.name} ({ctx.item_hash})")

    if missing_stage3i_tests or drifted_stage3i_tests:
        ctx.fail(
            "stage3i-runtime-evidence",
            "Stage3I tests must remain unconditional direct-parent #[test] functions with exact counts/blockers, raw8 typed chain and source/output order, no synthetic constant, strict/sloppy wires and realm semantics, all entrance-protocol negatives, raw8-absent compatibility, transactional rollback, and retry evidence; "
            f"missing {missing_stage3i_tests}, drifted {drifted_stage3i_tests}",
        )

    stage3j_test_contracts = deepcopy(evidence.STAGE3J_TEST_CONTRACTS)

    stage3j_test_body_hashes = deepcopy(evidence.STAGE3J_TEST_BODY_HASHES)

    stage3j_test_parent_bounds = dict(ctx.stage3d_test_parent_bounds)

    stage3j_test_sources = {relative for relative, _, _ in stage3j_test_contracts}

    for ctx.relative in stage3j_test_sources:
        ctx.code = ctx.stage3b_code(ctx.relative)
        if ctx.assertion_shadow.search(ctx.code):
            ctx.fail(
                "stage3j-runtime-evidence",
                f"{ctx.relative} must not shadow or import the assertion macros used by Stage3J evidence",
            )
        if re.search(r"(?m)^[ \t]*#![ \t]*\[[ \t]*(?:cfg|cfg_attr)\b", ctx.code):
            ctx.fail(
                "stage3j-runtime-evidence",
                f"{ctx.relative} must not conditionally exclude its Stage3J test evidence",
            )

    ctx.missing_stage3j_tests = []

    ctx.drifted_stage3j_tests = []

    for ctx.relative, ctx.name, ctx.anchors in stage3j_test_contracts:
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
            ctx.missing_stage3j_tests.append(f"{ctx.relative}::{ctx.name}")
            continue
        ctx.declaration_offset = ctx.declarations[0].start()
        ctx.declaration_depth = ctx.code[:ctx.declaration_offset].count("{") - ctx.code[:ctx.declaration_offset].count("}")
        if ctx.relative == "src/engine/heap/runtime/tests.rs":
            ctx.direct_parent = ctx.declaration_depth == 0
        else:
            ctx.parent_bounds = stage3j_test_parent_bounds.get(ctx.relative)
            ctx.direct_parent = (
                ctx.parent_bounds is not None
                and ctx.parent_bounds[0] < ctx.declaration_offset < ctx.parent_bounds[1]
                and ctx.declaration_depth == 1
            )
        if not ctx.direct_parent:
            ctx.missing_stage3j_tests.append(f"{ctx.relative}::{ctx.name} (nested)")
            continue
        ctx.item = ctx.stage3b_function(ctx.relative, ctx.name, "stage3j-runtime-evidence")
        ctx.normalized_item = " ".join(ctx.item.split())
        ctx.item_hash = ctx.normalized_code_sha256(ctx.item)
        if (
            any(anchor not in ctx.normalized_item for anchor in ctx.anchors)
            or ctx.item_hash != stage3j_test_body_hashes.get((ctx.relative, ctx.name))
        ):
            ctx.drifted_stage3j_tests.append(f"{ctx.relative}::{ctx.name} ({ctx.item_hash})")
