"""Runtime contracts checks, in the ordered boundary scan."""
from __future__ import annotations

import hashlib
import re
from copy import deepcopy

from ..evidence import runtime_contracts as evidence


def check(ctx):
    if ctx.self_test_marker_authorized:
        return

    stage3b_ordered_contracts = deepcopy(evidence.STAGE3B_ORDERED_CONTRACTS)

    for diagnostic, ctx.relative, ctx.function_name, fragments in stage3b_ordered_contracts:
        ctx.require_ordered_fragments(
            diagnostic,
            f"{ctx.relative}::{ctx.function_name} must retain its reviewed Stage3B branch order",
            ctx.stage3b_function(ctx.relative, ctx.function_name, diagnostic),
            fragments,
        )

    construct_dispatch = ctx.stage3b_function(
        "src/engine/heap/runtime/mod.rs", "construct_internal_with_new_target", "stage3b-raw-construction"
    )

    normalized_construct_dispatch = " ".join(construct_dispatch.split())

    if (
        normalized_construct_dispatch.count("new_target.value()") != 3
        or normalized_construct_dispatch.count("let raw_new_target") != 1
    ):
        ctx.fail("stage3b-raw-construction", "native, derived, and base paths must preserve one raw newTarget flow")

    apply_host = ctx.stage3b_function("src/engine/vm/host_bridge.rs", "apply", "stage3b-apply-order")

    if " ".join(apply_host.split()).count("build_argument_list(") != 1:
        ctx.fail("stage3b-apply-order", "Apply must build a nonnull argument list exactly once")

    realm_object_impl = ctx.stage3b_function(
        "src/engine/object/internal_methods.rs", "function_realm_object_impl", "stage3b-function-realm"
    )

    if " ".join(realm_object_impl.split()).count(
        "object = ObjectRef::from_borrowed_handle(self.clone(), target)?;"
    ) != 2:
        ctx.fail("stage3b-function-realm", "bound and Proxy realm traversal must each advance to their target")

    prototype_helper = ctx.stage3b_function(
        "src/engine/heap/runtime/mod.rs", "prototype_from_constructor_value", "stage3b-constructor-prototype"
    )

    if re.search(r"\b(?:CallableRef|callable_from_value|as_callable)\b", prototype_helper):
        ctx.fail("stage3b-constructor-prototype", "prototype fallback must consume raw newTarget")

    native_borrowed_prototype_consumers = (
        ("src/engine/heap/runtime/mod.rs", "create_from_constructor_value"),
        ("src/engine/builtins/array.rs", "create_array_from_constructor"),
    )

    native_owned_prototype_consumers = deepcopy(evidence.NATIVE_OWNED_PROTOTYPE_CONSUMERS)

    native_prototype_consumers = (
        *((relative, function_name, "new_target") for relative, function_name in native_borrowed_prototype_consumers),
        *((relative, function_name, "&new_target") for relative, function_name in native_owned_prototype_consumers),
    )

    for ctx.relative, ctx.function_name, new_target_argument in native_prototype_consumers:
        ctx.item = ctx.stage3b_function(ctx.relative, ctx.function_name, "stage3b-native-prototype-family")
        if (
            " ".join(ctx.item.split()).count("prototype_from_constructor_value(") != 1
            or " ".join(ctx.item.split()).count(f", {new_target_argument},") != 1
            or re.search(r"\b(?:CallableRef|callable_from_value)\b", ctx.item)
        ):
            ctx.fail("stage3b-native-prototype-family", f"{ctx.relative}::{ctx.function_name} bypasses the raw helper payload")

    constructor_only_items = deepcopy(evidence.CONSTRUCTOR_ONLY_ITEMS)

    for diagnostic, ctx.relative, ctx.function_name in constructor_only_items:
        if re.search(
            r"\b(?:CallableRef|callable_from_value|as_callable)\b",
            ctx.stage3b_function(ctx.relative, ctx.function_name, diagnostic),
        ):
            ctx.fail(diagnostic, f"{ctx.relative}::{ctx.function_name} must preserve constructor-only capability")

    context_construct = ctx.stage3b_function(
        "src/engine/api/context/calls.rs", "construct_with_new_target", "stage3b-public-construction"
    )

    if "raw_new_target" in " ".join(context_construct.split()):
        ctx.fail("stage3b-public-construction", "Context must not expose the raw VM construction seam")

    species_functions = deepcopy(evidence.SPECIES_FUNCTIONS)

    for ctx.relative, ctx.function_name, wraps_option in species_functions:
        ctx.item = ctx.stage3b_function(ctx.relative, ctx.function_name, "stage3b-species-constructor")
        ctx.normalized_item = " ".join(ctx.item.split())
        if (
            "ConstructorRef" not in ctx.normalized_item
            or ctx.normalized_item.count("constructor_from_value(") != 1
            or (
                wraps_option
                and ctx.normalized_item.count(
                    "NativeConversion::Value(constructor) => NativeConversion::Value(Some(constructor))"
                ) != 1
            )
            or re.search(r"\b(?:CallableRef|callable_from_value|as_callable)\b", ctx.item)
        ):
            ctx.fail("stage3b-species-constructor", f"{ctx.relative}::{ctx.function_name} narrows species capability")

    stage3b_runtime_test_contracts = deepcopy(evidence.STAGE3B_RUNTIME_TEST_CONTRACTS)

    runtime_tests_code = ctx.rust_code_only(ctx.read_source("src/engine/heap/runtime/tests.rs"))

    missing_stage3b_tests = []

    for ctx.name, ctx._ in stage3b_runtime_test_contracts:
        ctx.declarations = list(re.finditer(
            rf"\bfn[ \t\n]+{ctx.name}[ \t\n]*\(", runtime_tests_code
        ))
        if len(ctx.declarations) != 1:
            missing_stage3b_tests.append(ctx.name)
            continue
        declaration = ctx.declarations[0]
        previous_item_end = max(runtime_tests_code.rfind("}", 0, declaration.start()), runtime_tests_code.rfind(";", 0, declaration.start())) + 1
        attributes = runtime_tests_code[previous_item_end:declaration.start()]
        if " ".join(attributes.split()) != "#[test]":
            missing_stage3b_tests.append(ctx.name)

    drifted_stage3b_tests = []

    for ctx.name, ctx.anchors in stage3b_runtime_test_contracts:
        ctx.item = ctx.stage3b_function("src/engine/heap/runtime/tests.rs", ctx.name, "stage3b-runtime-evidence")
        ctx.normalized_item = " ".join(ctx.item.split())
        if any(anchor not in ctx.normalized_item for anchor in ctx.anchors):
            drifted_stage3b_tests.append(ctx.name)

    if (
        missing_stage3b_tests
        or drifted_stage3b_tests
        or runtime_tests_code.count("for magic in [2_u16, u16::MAX]") != 1
        or ctx.function_translate_code.count("for magic in [2, u16::MAX]") != 1
    ):
        ctx.fail(
            "stage3b-runtime-evidence",
            "Stage3B must retain its canonical/noncanonical, ordering, raw propagation, realm, native-family, Proxy, stack, and branch regression matrix; "
            f"missing {missing_stage3b_tests}, drifted {drifted_stage3b_tests}",
        )

    stage3c_test_module_contracts = deepcopy(evidence.STAGE3C_TEST_MODULE_CONTRACTS)

    test_module_pattern = re.compile(
        r"(?m)(?P<attributes>(?:^[ \t]*#[ \t]*\[[^]\n]*\][ \t]*\n)*)"
        r"^[ \t]*mod[ \t]+tests[ \t]*(?P<form>;|\{)"
    )

    drifted_stage3c_test_modules = []

    for ctx.relative, expected_form in stage3c_test_module_contracts:
        ctx.code = ctx.stage3b_code(ctx.relative)
        ctx.declarations = [
            declaration
            for declaration in test_module_pattern.finditer(ctx.code)
            if ctx.code[:declaration.start()].count("{")
            == ctx.code[:declaration.start()].count("}")
        ]
        if (
            len(ctx.declarations) != 1
            or " ".join(ctx.declarations[0].group("attributes").split()) != "#[cfg(test)]"
            or ctx.declarations[0].group("form") != expected_form
        ):
            drifted_stage3c_test_modules.append(ctx.relative)

    stage3c_required_test_sources = deepcopy(evidence.STAGE3C_REQUIRED_TEST_SOURCES)

    for ctx.relative in stage3c_required_test_sources:
        if re.search(
            r"(?m)^[ \t]*#![ \t]*\[[ \t]*(?:cfg|cfg_attr)\b",
            ctx.stage3b_code(ctx.relative),
        ):
            drifted_stage3c_test_modules.append(ctx.relative)

    if drifted_stage3c_test_modules:
        ctx.fail(
            "stage3c-runtime-evidence",
            "every required Stage3C test module must retain exactly #[cfg(test)] and no outer or inner cfg/cfg_attr exclusion; "
            f"drifted {sorted(set(drifted_stage3c_test_modules))}",
        )

    stage3c_test_contracts = deepcopy(evidence.STAGE3C_TEST_CONTRACTS)

    stage3c_test_body_hashes = deepcopy(evidence.STAGE3C_TEST_BODY_HASHES)

    missing_stage3c_tests = []

    drifted_stage3c_tests = []

    for ctx.relative, ctx.name, ctx.anchors in stage3c_test_contracts:
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
            missing_stage3c_tests.append(f"{ctx.relative}::{ctx.name}")
            continue
        ctx.item = ctx.stage3b_function(ctx.relative, ctx.name, "stage3c-runtime-evidence")
        ctx.normalized_item = " ".join(ctx.item.split())
        if (
            any(anchor not in ctx.normalized_item for anchor in ctx.anchors)
            or ctx.normalized_code_sha256(ctx.item)
            != stage3c_test_body_hashes.get((ctx.relative, ctx.name))
        ):
            drifted_stage3c_tests.append(f"{ctx.relative}::{ctx.name}")

    if missing_stage3c_tests or drifted_stage3c_tests:
        ctx.fail(
            "stage3c-runtime-evidence",
            "Stage3C tests must retain exact #[test] attributes and the typed-chain, BC5, verifier, operand-order, completion, catch, backtrace, unwind, recovery, and rollback evidence; "
            f"missing {missing_stage3c_tests}, drifted {drifted_stage3c_tests}",
        )

    stage3d_runtime_tests_code = ctx.stage3b_code("src/engine/heap/runtime/tests.rs")

    stage3d_throw_wire_matches = list(re.finditer(
        r"\bconst[ \t\n]+QUICKJS_ORDINARY_THROW_BC5[ \t\n]*:"
        r"[ \t\n]*&[ \t\n]*\[[ \t\n]*u8[ \t\n]*\][ \t\n]*="
        r"[ \t\n]*&[ \t\n]*\[(?P<body>[^]]*)\][ \t\n]*;",
        stage3d_runtime_tests_code,
    ))

    stage3d_throw_wire = b""

    if len(stage3d_throw_wire_matches) != 1:
        ctx.fail(
            "stage3d-runtime-evidence",
            "the Rust runtime evidence must retain exactly one literal QUICKJS_ORDINARY_THROW_BC5 wire",
        )
    else:
        wire_body = stage3d_throw_wire_matches[0].group("body")
        wire_tokens = re.findall(r"0x[0-9A-Fa-f]+|[0-9]+", wire_body)
        wire_residue = re.sub(r"0x[0-9A-Fa-f]+|[0-9]+|[\s,]", "", wire_body)
        try:
            stage3d_throw_wire = bytes(int(token, 0) for token in wire_tokens)
        except ValueError:
            wire_residue = "invalid-byte"
        ctx.fnv = 0xCBF29CE484222325
        for byte in stage3d_throw_wire:
            ctx.fnv ^= byte
            ctx.fnv = (ctx.fnv * 0x100000001B3) & 0xFFFFFFFFFFFFFFFF
        if (
            wire_residue
            or len(stage3d_throw_wire) != 45
            or ctx.fnv != 0x73CF217E06C5FEE2
            or hashlib.sha256(stage3d_throw_wire).hexdigest()
            != "b7998b9678635e7e0a4eb2e465b683d168395adc7f156f733c25521907e3c8a8"
            or stage3d_throw_wire[-2:] != bytes((0xCF, 0x30))
        ):
            ctx.fail(
                "stage3d-runtime-evidence",
                "the Rust raw48 fixture must remain the exact 45-byte cf30 wire with its frozen FNV-1a-64 and SHA-256",
            )
