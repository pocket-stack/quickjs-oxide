"""Exception evidence checks, in the ordered boundary scan."""
from __future__ import annotations

import re
from copy import deepcopy

from ..evidence import exception_evidence as evidence


def check(ctx):
    if ctx.self_test_marker_authorized:
        return

    stage3d_test_contracts = deepcopy(evidence.STAGE3D_TEST_CONTRACTS)

    stage3d_test_body_hashes = deepcopy(evidence.STAGE3D_TEST_BODY_HASHES)

    stage3d_inline_test_files = {
        relative
        for relative, _, _ in stage3d_test_contracts
        if relative != "src/engine/heap/runtime/tests.rs"
    }

    ctx.stage3d_test_parent_bounds: dict[str, tuple[int, int]] = {}

    for ctx.relative in stage3d_inline_test_files:
        ctx.code = ctx.stage3b_code(ctx.relative)
        modules = list(re.finditer(
            r"(?P<attributes>(?:#[ \t\n]*\[[^]]*\][ \t\n]*)*)"
            r"\bmod[ \t\n]+tests[ \t\n]*\{",
            ctx.code,
        ))
        if (
            len(modules) != 1
            or " ".join(modules[0].group("attributes").split()) != "#[cfg(test)]"
        ):
            ctx.fail(
                "stage3d-runtime-evidence",
                f"{ctx.relative} must retain one direct, unconditional #[cfg(test)] tests module",
            )
            continue
        ctx._, module_start, module_end = ctx.braced_item_from_match(
            ctx.code,
            modules[0],
            "stage3d-runtime-evidence",
            f"{ctx.relative} direct tests module",
        )
        ctx.stage3d_test_parent_bounds[ctx.relative] = (module_start, module_end)

    ctx.assertion_shadow = ctx.assertion_shadow_pattern

    for ctx.relative in stage3d_inline_test_files | {"src/engine/heap/runtime/tests.rs"}:
        if ctx.assertion_shadow.search(ctx.stage3b_code(ctx.relative)):
            ctx.fail(
                "stage3d-runtime-evidence",
                f"{ctx.relative} must not shadow or import the assertion macros used by Stage3D evidence",
            )

    missing_stage3d_tests = []

    drifted_stage3d_tests = []

    for ctx.relative, ctx.name, ctx.anchors in stage3d_test_contracts:
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
            missing_stage3d_tests.append(f"{ctx.relative}::{ctx.name}")
            continue
        ctx.declaration_offset = ctx.declarations[0].start()
        ctx.declaration_depth = (
            ctx.code[:ctx.declaration_offset].count("{")
            - ctx.code[:ctx.declaration_offset].count("}")
        )
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
            missing_stage3d_tests.append(f"{ctx.relative}::{ctx.name} (nested)")
            continue
        ctx.item = ctx.stage3b_function(ctx.relative, ctx.name, "stage3d-runtime-evidence")
        ctx.normalized_item = " ".join(ctx.item.split())
        if (
            any(anchor not in ctx.normalized_item for anchor in ctx.anchors)
            or ctx.normalized_code_sha256(ctx.item)
            != stage3d_test_body_hashes.get((ctx.relative, ctx.name))
        ):
            drifted_stage3d_tests.append(f"{ctx.relative}::{ctx.name}")

    if missing_stage3d_tests or drifted_stage3d_tests:
        ctx.fail(
            "stage3d-runtime-evidence",
            "Stage3D tests must remain unconditional #[test] functions with the exact raw48 wire, typed chain, terminal verifier, identity, pending, catch, backtrace, iterator-close, recovery, and rollback evidence; "
            f"missing {missing_stage3d_tests}, drifted {drifted_stage3d_tests}",
        )

    stage3e_test_contracts = deepcopy(evidence.STAGE3E_TEST_CONTRACTS)

    stage3e_test_body_hashes = deepcopy(evidence.STAGE3E_TEST_BODY_HASHES)

    missing_stage3e_tests = []

    drifted_stage3e_tests = []

    for ctx.relative, ctx.name, ctx.anchors in stage3e_test_contracts:
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
            missing_stage3e_tests.append(f"{ctx.relative}::{ctx.name}")
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
            missing_stage3e_tests.append(f"{ctx.relative}::{ctx.name} (nested)")
            continue
        ctx.item = ctx.stage3b_function(ctx.relative, ctx.name, "stage3e-runtime-evidence")
        ctx.normalized_item = " ".join(ctx.item.split())
        if (
            any(anchor not in ctx.normalized_item for anchor in ctx.anchors)
            or ctx.normalized_code_sha256(ctx.item)
            != stage3e_test_body_hashes.get((ctx.relative, ctx.name))
        ):
            drifted_stage3e_tests.append(f"{ctx.relative}::{ctx.name}")

    if missing_stage3e_tests or drifted_stage3e_tests:
        ctx.fail(
            "stage3e-runtime-evidence",
            "Stage3E tests must remain unconditional direct-parent #[test] functions with exact raw49 wires, subtype/atom provenance, synthetic String, zero-stack terminal, realm, backtrace, pending, catch, retry, and rollback evidence; "
            f"missing {missing_stage3e_tests}, drifted {drifted_stage3e_tests}",
        )

    stage3f_test_contracts = deepcopy(evidence.STAGE3F_TEST_CONTRACTS)

    stage3f_test_body_hashes = deepcopy(evidence.STAGE3F_TEST_BODY_HASHES)

    missing_stage3f_tests = []

    drifted_stage3f_tests = []

    for ctx.relative, ctx.name, ctx.anchors in stage3f_test_contracts:
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
            missing_stage3f_tests.append(f"{ctx.relative}::{ctx.name}")
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
            missing_stage3f_tests.append(f"{ctx.relative}::{ctx.name} (nested)")
            continue
        ctx.item = ctx.stage3b_function(ctx.relative, ctx.name, "stage3f-runtime-evidence")
        ctx.normalized_item = " ".join(ctx.item.split())
        if (
            any(anchor not in ctx.normalized_item for anchor in ctx.anchors)
            or ctx.normalized_code_sha256(ctx.item)
            != stage3f_test_body_hashes.get((ctx.relative, ctx.name))
        ):
            drifted_stage3f_tests.append(
                f"{ctx.relative}::{ctx.name} ({ctx.normalized_code_sha256(ctx.item)})"
            )

    if missing_stage3f_tests or drifted_stage3f_tests:
        ctx.fail(
            "stage3f-runtime-evidence",
            "Stage3F tests must remain unconditional direct-parent #[test] functions with the exact raw177 registry/typed chain, 41-byte runtime metadata/realm/pending evidence, 40-byte fallthrough rollback/retry, and branch-index preservation; "
            f"missing {missing_stage3f_tests}, drifted {drifted_stage3f_tests}",
        )
