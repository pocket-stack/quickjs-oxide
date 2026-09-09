"""Surface checks, in the ordered boundary scan."""
from __future__ import annotations

from pathlib import Path
import re
import tomllib
from copy import deepcopy

from ..evidence import surface as evidence


def check(ctx):
    self_test_marker = ctx.root / ".boundary-self-test"

    self_test_marker_exists = self_test_marker.exists() or self_test_marker.is_symlink()

    ctx.self_test_marker_authorized = (
        bool(ctx.self_test_token)
        and self_test_marker.is_file()
        and not self_test_marker.is_symlink()
        and self_test_marker.stat().st_nlink == 1
        and self_test_marker.read_text(encoding="utf-8") == f"{ctx.self_test_token}\n"
    )

    if self_test_marker_exists and not ctx.self_test_marker_authorized:
        ctx.fail(
            "boundary-self-test-marker",
            ".boundary-self-test is reserved for the gate's out-of-band authorized reduced fixture and is forbidden in a real scan root",
        )

    if ctx.self_test_token and not ctx.self_test_marker_authorized:
        ctx.fail(
            "boundary-self-test-marker",
            "the out-of-band reduced-fixture token must match one regular, single-link .boundary-self-test marker",
        )

    if not ctx.self_test_marker_authorized:
        cargo_source = ctx.read_source("Cargo.toml")
        try:
            cargo_manifest = tomllib.loads(cargo_source)
        except tomllib.TOMLDecodeError as error:
            ctx.fail("stage3e-test-target", f"Cargo.toml must remain valid TOML: {error}")
            cargo_manifest = {}
        if cargo_manifest.get("lib", {}) != {} or cargo_manifest.get("package", {}).get("name") != "quickjs-oxide":
            ctx.fail(
                "stage3e-test-target",
                "engine Cargo.toml must retain its conventional src/lib.rs target without test/harness overrides",
            )

    lib_source = ctx.read_source("src/lib.rs")

    lib_code = ctx.rust_code_only(lib_source)

    if not ctx.self_test_marker_authorized:
        ctx.require_normalized_code_sha256(
            "stage3e-runtime-evidence",
            "src/lib.rs must retain its exact candidate-A module and embedding API routing",
            lib_code,
            "4de31aa48efb93b081c9948a45c22c64a9816aaa3eeed58d734f2e9c8031892b",
        )

    for ctx.match in re.finditer(r"\bbinary_object\b", lib_source):
        ctx.fail(
            "public-lib-boundary",
            "src/lib.rs must not name binary_object; found "
            + ctx.location("src/lib.rs", lib_source, ctx.match.start()),
        )

    if re.search(
        r"(?m)^[ \t]*#![ \t]*\[[ \t]*(?:cfg|cfg_attr)\b",
        lib_code,
    ):
        ctx.fail(
            "stage3e-runtime-evidence",
            "src/lib.rs must not conditionally exclude the crate or its unit-test target with an inner cfg/cfg_attr",
        )

    ctx.assertion_shadow_pattern = re.compile(
        r"\bmacro_rules[ \t\n]*![ \t\n]*(?:r#)?(?:assert|assert_eq|assert_ne|matches|panic)\b"
        r"|^[ \t]*(?:(?:pub(?:[ \t]*\([^)]*\))?)[ \t]+)?use[ \t]+[^;]*"
        r"\b(?:r#)?(?:assert|assert_eq|assert_ne|matches|panic)\b[^;]*;",
        re.MULTILINE,
    )

    if ctx.assertion_shadow_pattern.search(lib_code):
        ctx.fail(
            "stage3e-runtime-evidence",
            "src/lib.rs must not shadow or import the assertion macros used by Stage3E unit-test evidence",
        )

    runtime_source = ctx.read_source("src/engine/heap/runtime/mod.rs")

    ctx.runtime_code = ctx.rust_code_only(runtime_source)

    codec_parent = ctx.read_source("src/engine/code/mod.rs")
    codec_parent_code = ctx.rust_code_only(codec_parent)
    runtime_mentions = list(re.finditer(r"\bbinary_object\b", codec_parent))

    private_declarations = re.findall(
        r"(?m)^[ \t]*mod[ \t]+binary_object[ \t]*;[ \t]*$", codec_parent_code
    )

    if len(private_declarations) != 1:
        ctx.fail(
            "runtime-private-module",
            "src/engine/code/mod.rs must contain exactly one private `mod binary_object;` declaration",
        )

    if len(runtime_mentions) != 1:
        details = ", ".join(
            ctx.location("src/engine/heap/runtime/mod.rs", runtime_source, match.start())
            for match in runtime_mentions
        )
        ctx.fail(
            "runtime-boundary",
            "src/engine/code/mod.rs may name binary_object only in its private module declaration"
            + (f"; found {details}" if details else ""),
        )

    ctx.binary_root_relative = "src/engine/code/binary_object/mod.rs"

    binary_root_source = ctx.read_source(ctx.binary_root_relative)

    binary_root_code = ctx.rust_code_only(binary_root_source)

    expected_root_modules = deepcopy(evidence.EXPECTED_ROOT_MODULES)

    for module in expected_root_modules:
        ctx.declarations = re.findall(
            rf"(?m)^[ \t]*mod[ \t]+{re.escape(module)}[ \t]*;[ \t]*$",
            binary_root_code,
        )
        if len(ctx.declarations) != 1:
            ctx.fail(
                "root-private-module",
                f"{ctx.binary_root_relative} must contain exactly one private `mod {module};` declaration",
            )

    root_module_declarations = re.findall(
        r"(?m)^[ \t]*mod[ \t]+([A-Za-z_][A-Za-z0-9_]*)[ \t]*;[ \t]*$",
        binary_root_code,
    )

    if sorted(root_module_declarations) != sorted(expected_root_modules):
        ctx.fail(
            "root-private-module-set",
            f"{ctx.binary_root_relative} must contain only the reviewed private module set; "
            f"found {root_module_declarations}",
        )

    ctx.public_module_pattern = re.compile(
        r"(?m)^[ \t]*pub(?:[ \t\n]*\([^)]*\))?[ \t\n]+mod[ \t\n]+[A-Za-z_][A-Za-z0-9_]*"
    )

    for ctx.match in ctx.public_module_pattern.finditer(binary_root_code):
        ctx.fail(
            "root-module-visibility",
            "binary_object root submodules must remain private; found "
            + ctx.location(ctx.binary_root_relative, binary_root_source, ctx.match.start()),
        )

    public_use_pattern = re.compile(
        r"(?m)^[ \t]*pub(?:[ \t\n]*\([^)]*\))?[ \t\n]+use\b"
    )

    scalar_facade_pattern = re.compile(
        r"(?m)^[ \t]*pub[ \t\n]*\([ \t\n]*super[ \t\n]*\)[ \t\n]+use"
        r"[ \t\n]+scalar_script[ \t\n]*::[ \t\n]*\{(?P<body>[^{}]*)\}"
        r"[ \t\n]*;"
    )

    scalar_facades = list(scalar_facade_pattern.finditer(binary_root_code))

    ctx.expected_scalar_facade_names = {
        "ScalarScriptReadError",
        "ScalarStringDraft",
        "ScalarUnaryOp",
        "ScalarValueDraft",
        "decode_trusted_scalar_script",
    }

    facade_names: set[str] = set()

    if len(scalar_facades) == 1:
        facade_items = [
            item.strip() for item in scalar_facades[0].group("body").split(",") if item.strip()
        ]
        facade_names = set(facade_items)
        if (
            len(facade_items) != len(ctx.expected_scalar_facade_names)
            or facade_names != ctx.expected_scalar_facade_names
        ):
            ctx.fail(
                "scalar-script-facade-shape",
                "binary_object must expose exactly the reviewed scalar-script facade names; "
                f"found {facade_items}",
            )
    else:
        ctx.fail(
            "scalar-script-facade-shape",
            "binary_object must contain exactly one private-parent scalar-script facade re-export",
        )

    ordinary_facade_pattern = re.compile(
        r"(?m)^[ \t]*pub[ \t\n]*\([ \t\n]*super[ \t\n]*\)[ \t\n]+use"
        r"[ \t\n]+ordinary_leaf[ \t\n]*::[ \t\n]*\{(?P<body>[^{}]*)\}"
        r"[ \t\n]*;"
    )

    ordinary_facades = list(ordinary_facade_pattern.finditer(binary_root_code))

    expected_ordinary_facade_names = deepcopy(evidence.EXPECTED_ORDINARY_FACADE_NAMES)

    if len(ordinary_facades) == 1:
        ordinary_facade_items = [
            item.strip()
            for item in ordinary_facades[0].group("body").split(",")
            if item.strip()
        ]
        if (
            len(ordinary_facade_items) != len(expected_ordinary_facade_names)
            or set(ordinary_facade_items) != expected_ordinary_facade_names
        ):
            ctx.fail(
                "ordinary-leaf-facade-shape",
                "binary_object must expose exactly the reviewed ordinary-leaf facade names; "
                f"found {ordinary_facade_items}",
            )
    else:
        ctx.fail(
            "ordinary-leaf-facade-shape",
            "binary_object must contain exactly one private-parent ordinary-leaf facade re-export",
        )

    facade_offsets = {
        match.start() for match in (*scalar_facades, *ordinary_facades)
    }

    for ctx.match in public_use_pattern.finditer(binary_root_code):
        if ctx.match.start() in facade_offsets:
            continue
        ctx.fail(
            "root-reexport",
            "binary_object root may re-export only the reviewed scalar-script and ordinary-leaf facades; found "
            + ctx.location(ctx.binary_root_relative, binary_root_source, ctx.match.start()),
        )

    ctx.image_root_relative = "src/engine/code/binary_object/bytecode_image/mod.rs"

    image_root_source = ctx.read_source(ctx.image_root_relative)

    ctx.image_root_code = ctx.rust_code_only(image_root_source)

    expected_image_modules = (
        "atoms",
        "budget",
        "decode",
        "encode",
        "model",
        "native_plan",
        "tests",
    )

    for module in expected_image_modules:
        ctx.declarations = re.findall(
            rf"(?m)^[ \t]*mod[ \t]+{re.escape(module)}[ \t]*;[ \t]*$",
            ctx.image_root_code,
        )
        if len(ctx.declarations) != 1:
            ctx.fail(
                "image-private-module",
                f"{ctx.image_root_relative} must contain exactly one private `mod {module};` declaration",
            )

    image_module_declarations = re.findall(
        r"(?m)^[ \t]*mod[ \t]+([A-Za-z_][A-Za-z0-9_]*)[ \t]*;[ \t]*$",
        ctx.image_root_code,
    )

    if sorted(image_module_declarations) != sorted(expected_image_modules):
        ctx.fail(
            "image-private-module",
            f"{ctx.image_root_relative} must contain only the reviewed private module set; "
            f"found {image_module_declarations}",
        )

    for ctx.match in ctx.public_module_pattern.finditer(ctx.image_root_code):
        ctx.fail(
            "image-module-visibility",
            "bytecode_image submodules must remain private; found "
            + ctx.location(ctx.image_root_relative, image_root_source, ctx.match.start()),
        )

    binary_root = ctx.root / "src/engine/code/binary_object"

    if binary_root.is_symlink() or not binary_root.is_dir():
        ctx.fail("missing-source", "src/engine/code/binary_object must be a regular directory")
        ctx.binary_sources: list[Path] = []
    else:
        ctx.binary_sources = sorted(binary_root.rglob("*.rs"))

    ctx.binary_source_cache = {
        path: path.read_text(encoding="utf-8")
        for path in ctx.binary_sources
        if not path.is_symlink() and path.is_file()
    }

    ctx.binary_code_cache = {
        path: ctx.rust_code_only(source) for path, source in ctx.binary_source_cache.items()
    }

    expected_binary_visible_counts = deepcopy(evidence.EXPECTED_BINARY_VISIBLE_COUNTS)

    expected_fixture_visible_counts = deepcopy(evidence.EXPECTED_FIXTURE_VISIBLE_COUNTS)

    binary_visible_counts = {
        path.relative_to(ctx.root).as_posix(): len(
            re.findall(r"\bpub(?:[ \t\n]*\([^)]*\))?", code)
        )
        for path, code in ctx.binary_code_cache.items()
        if not ctx.is_test_source(path)
        and re.search(r"\bpub(?:[ \t\n]*\([^)]*\))?", code)
    }

    ctx.is_full_binary_inventory = binary_visible_counts == expected_binary_visible_counts

    if binary_visible_counts not in (
        expected_binary_visible_counts,
        expected_fixture_visible_counts,
    ):
        ctx.fail(
            "binary-object-visible-surface",
            "binary_object production visibility counts drifted from the reviewed module surface; "
            f"found {binary_visible_counts}",
        )

    bytecode_image_impl_header_pattern = re.compile(
        r"(?m)^[ \t]*impl\b(?P<header>[^{};]*)\{"
    )

    bytecode_image_impl_headers = {
        path.relative_to(ctx.root).as_posix(): [
            " ".join(match.group("header").split())
            for match in bytecode_image_impl_header_pattern.finditer(code)
        ]
        for path, code in ctx.binary_code_cache.items()
        if not ctx.is_test_source(path)
        and path.relative_to(ctx.root).as_posix().startswith(
            "src/engine/code/binary_object/bytecode_image/"
        )
        and bytecode_image_impl_header_pattern.search(code)
    }

    expected_bytecode_image_impl_headers = deepcopy(evidence.EXPECTED_BYTECODE_IMAGE_IMPL_HEADERS)

    expected_fixture_bytecode_image_impl_headers = deepcopy(evidence.EXPECTED_FIXTURE_BYTECODE_IMAGE_IMPL_HEADERS)

    if bytecode_image_impl_headers not in (
        expected_bytecode_image_impl_headers,
        expected_fixture_bytecode_image_impl_headers,
    ):
        ctx.fail(
            "bytecode-image-implementation-set",
            "bytecode_image implementation ownership drifted from the reviewed inherent and trait set; "
            f"found {bytecode_image_impl_headers}",
        )
