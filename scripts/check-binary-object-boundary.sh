#!/usr/bin/env bash
# Keep the release-pinned binary-object archive codec isolated from the
# compiler, VM, runtime publication path, and public crate surface.

set -euo pipefail
export LC_ALL=C

script_dir=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd)
repository_root=$(CDPATH='' cd -- "$script_dir/.." && pwd)

die() {
    echo "error: $*" >&2
    exit 1
}

command -v python3 >/dev/null 2>&1 || die "python3 is required"

scan_root() {
    local candidate_root=$1
    local root

    root=$(CDPATH='' cd -- "$candidate_root" && pwd)
    python3 - "$root" <<'PY'
from __future__ import annotations

from pathlib import Path
import hashlib
import html
import re
import sys
import tomllib
import unicodedata


root = Path(sys.argv[1])
errors: list[str] = []


def fail(code: str, message: str) -> None:
    errors.append(f"{code}: {message}")


def read_source(relative: str) -> str:
    path = root / relative
    if path.is_symlink() or not path.is_file():
        fail("missing-source", f"{relative} must be a regular file")
        return ""
    return path.read_text(encoding="utf-8")


def blank(text: str) -> str:
    return "".join("\n" if character == "\n" else " " for character in text)


raw_string_prefix = re.compile(r'(?:br|rb|cr|rc|r)(?P<hashes>#{0,255})"')


def rust_code_only(source: str) -> str:
    """Remove comments and strings while retaining offsets and line numbers."""
    output: list[str] = []
    index = 0
    length = len(source)
    while index < length:
        if source.startswith("//", index):
            end = source.find("\n", index)
            if end < 0:
                end = length
            output.append(blank(source[index:end]))
            index = end
            continue

        if source.startswith("/*", index):
            start = index
            depth = 1
            index += 2
            while index < length and depth:
                if source.startswith("/*", index):
                    depth += 1
                    index += 2
                elif source.startswith("*/", index):
                    depth -= 1
                    index += 2
                else:
                    index += 1
            output.append(blank(source[start:index]))
            continue

        # Match against the original buffer at an absolute offset. Slicing the
        # entire remaining source at every byte turns large-file scans into an
        # accidental quadratic operation.
        raw = raw_string_prefix.match(source, index)
        if raw is not None:
            start = index
            hashes = raw.group("hashes")
            index = raw.end()
            terminator = '"' + hashes
            end = source.find(terminator, index)
            index = length if end < 0 else end + len(terminator)
            output.append(blank(source[start:index]))
            continue

        quote_offset = 1 if source[index:index + 2] in {'b"', 'c"'} else 0
        if source[index + quote_offset:index + quote_offset + 1] == '"':
            start = index
            index += quote_offset + 1
            while index < length:
                if source[index] == "\\":
                    index = min(length, index + 2)
                elif source[index] == '"':
                    index += 1
                    break
                else:
                    index += 1
            output.append(blank(source[start:index]))
            continue

        output.append(source[index])
        index += 1

    return "".join(output)


def location(relative: str, source: str, offset: int) -> str:
    line = source.count("\n", 0, offset) + 1
    line_start = source.rfind("\n", 0, offset) + 1
    line_end = source.find("\n", offset)
    if line_end < 0:
        line_end = len(source)
    excerpt = source[line_start:line_end].strip()
    return f"{relative}:{line}: {excerpt}"


def normalized_code_sha256(code: str) -> str:
    return hashlib.sha256(" ".join(code.split()).encode("utf-8")).hexdigest()


def require_normalized_code_sha256(
    code_name: str,
    description: str,
    code: str,
    expected: str,
) -> None:
    found = normalized_code_sha256(code)
    if found != expected:
        fail(code_name, f"{description}; normalized code sha256 {found}")


def require_ordered_fragments(
    code_name: str,
    description: str,
    code: str,
    fragments: tuple[str, ...],
) -> None:
    normalized = " ".join(code.split())
    offsets = [normalized.find(fragment) for fragment in fragments]
    if (
        any(offset < 0 for offset in offsets)
        or offsets != sorted(offsets)
        or any(normalized.count(fragment) != 1 for fragment in fragments)
    ):
        fail(code_name, description)


def require_normalized_corridor_sha256(
    code_name: str,
    description: str,
    code: str,
    start_fragment: str,
    end_fragment: str,
    expected: str,
) -> None:
    normalized = " ".join(code.split())
    if (
        normalized.count(start_fragment) != 1
        or normalized.count(end_fragment) != 1
    ):
        fail(code_name, description)
        return
    start = normalized.find(start_fragment)
    end_start = normalized.find(end_fragment, start)
    if end_start < start:
        fail(code_name, description)
        return
    corridor = normalized[start:end_start + len(end_fragment)]
    require_normalized_code_sha256(
        code_name,
        description,
        corridor,
        expected,
    )


def unique_braced_item(
    code: str,
    pattern: re.Pattern[str],
    error_code: str,
    description: str,
) -> tuple[str, int, int]:
    matches = list(pattern.finditer(code))
    if len(matches) != 1:
        fail(error_code, f"must contain exactly one {description}")
        return "", -1, -1

    return braced_item_from_match(code, matches[0], error_code, description)


def braced_item_from_match(
    code: str,
    item: re.Match[str],
    error_code: str,
    description: str,
) -> tuple[str, int, int]:
    depth = 0
    for offset in range(item.end() - 1, len(code)):
        if code[offset] == "{":
            depth += 1
        elif code[offset] == "}":
            depth -= 1
            if depth == 0:
                return code[item.start():offset + 1], item.start(), offset + 1
    fail(error_code, f"{description} has no balanced closing brace")
    return "", -1, -1


def rustfmt_match_arms(item: str, prefix: str) -> list[tuple[str, str]]:
    """Return normalized top-level arms from a rustfmt-indented match."""
    pattern = re.compile(
        rf"(?ms)^        (?P<lhs>{re.escape(prefix)}.*?) => (?P<rhs>.*?)"
        rf"(?=^        (?:{re.escape(prefix)}|_ =>)|^    \}})"
    )
    return [
        (
            " ".join(match.group("lhs").split()),
            " ".join(match.group("rhs").rstrip(", \n").split()),
        )
        for match in pattern.finditer(item)
    ]


if not (root / ".boundary-self-test").is_file():
    cargo_source = read_source("Cargo.toml")
    try:
        cargo_manifest = tomllib.loads(cargo_source)
    except tomllib.TOMLDecodeError as error:
        fail("stage3e-test-target", f"Cargo.toml must remain valid TOML: {error}")
        cargo_manifest = {}
    if cargo_manifest.get("lib") != {
        "name": "quickjs_oxide",
        "path": "src/lib.rs",
    }:
        fail(
            "stage3e-test-target",
            "Cargo.toml [lib] must remain the exact src/lib.rs test target with no test/harness disable or path reroute",
        )

lib_source = read_source("src/lib.rs")
lib_code = rust_code_only(lib_source)
if not (root / ".boundary-self-test").is_file():
    require_normalized_code_sha256(
        "stage3e-runtime-evidence",
        "src/lib.rs must retain its exact crate/test routing without macro-use, path, include, or glob-import indirection",
        lib_code,
        "b3af80b7d2571f798e5408f959193827175248db16c9fbf137c423ca0523551d",
    )
for match in re.finditer(r"\bbinary_object\b", lib_source):
    fail(
        "public-lib-boundary",
        "src/lib.rs must not name binary_object; found "
        + location("src/lib.rs", lib_source, match.start()),
    )
if re.search(
    r"(?m)^[ \t]*#![ \t]*\[[ \t]*(?:cfg|cfg_attr)\b",
    lib_code,
):
    fail(
        "stage3e-runtime-evidence",
        "src/lib.rs must not conditionally exclude the crate or its unit-test target with an inner cfg/cfg_attr",
    )
assertion_shadow_pattern = re.compile(
    r"\bmacro_rules[ \t\n]*![ \t\n]*(?:r#)?(?:assert|assert_eq|assert_ne|matches|panic)\b"
    r"|^[ \t]*(?:(?:pub(?:[ \t]*\([^)]*\))?)[ \t]+)?use[ \t]+[^;]*"
    r"\b(?:r#)?(?:assert|assert_eq|assert_ne|matches|panic)\b[^;]*;",
    re.MULTILINE,
)
if assertion_shadow_pattern.search(lib_code):
    fail(
        "stage3e-runtime-evidence",
        "src/lib.rs must not shadow or import the assertion macros used by Stage3E unit-test evidence",
    )

runtime_source = read_source("src/runtime.rs")
runtime_code = rust_code_only(runtime_source)
runtime_mentions = list(re.finditer(r"\bbinary_object\b", runtime_source))
private_declarations = re.findall(
    r"(?m)^[ \t]*mod[ \t]+binary_object[ \t]*;[ \t]*$", runtime_code
)
if len(private_declarations) != 1:
    fail(
        "runtime-private-module",
        "src/runtime.rs must contain exactly one private `mod binary_object;` declaration",
    )
if len(runtime_mentions) != 1:
    details = ", ".join(
        location("src/runtime.rs", runtime_source, match.start())
        for match in runtime_mentions
    )
    fail(
        "runtime-boundary",
        "src/runtime.rs may name binary_object only in its private module declaration"
        + (f"; found {details}" if details else ""),
    )

binary_root_relative = "src/runtime/binary_object/mod.rs"
binary_root_source = read_source(binary_root_relative)
binary_root_code = rust_code_only(binary_root_source)
expected_root_modules = (
    "atoms",
    "code",
    "function_envelope",
    "function_translate",
    "bytecode_image",
    "graph",
    "pinned_atoms",
    "pinned_opcodes",
    "read_cursor",
    "ordinary_leaf",
    "scalar_script",
    "wire",
)
for module in expected_root_modules:
    declarations = re.findall(
        rf"(?m)^[ \t]*mod[ \t]+{re.escape(module)}[ \t]*;[ \t]*$",
        binary_root_code,
    )
    if len(declarations) != 1:
        fail(
            "root-private-module",
            f"{binary_root_relative} must contain exactly one private `mod {module};` declaration",
        )

root_module_declarations = re.findall(
    r"(?m)^[ \t]*mod[ \t]+([A-Za-z_][A-Za-z0-9_]*)[ \t]*;[ \t]*$",
    binary_root_code,
)
if sorted(root_module_declarations) != sorted(expected_root_modules):
    fail(
        "root-private-module-set",
        f"{binary_root_relative} must contain only the reviewed private module set; "
        f"found {root_module_declarations}",
    )

public_module_pattern = re.compile(
    r"(?m)^[ \t]*pub(?:[ \t\n]*\([^)]*\))?[ \t\n]+mod[ \t\n]+[A-Za-z_][A-Za-z0-9_]*"
)
for match in public_module_pattern.finditer(binary_root_code):
    fail(
        "root-module-visibility",
        "binary_object root submodules must remain private; found "
        + location(binary_root_relative, binary_root_source, match.start()),
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
expected_scalar_facade_names = {
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
        len(facade_items) != len(expected_scalar_facade_names)
        or facade_names != expected_scalar_facade_names
    ):
        fail(
            "scalar-script-facade-shape",
            "binary_object must expose exactly the reviewed scalar-script facade names; "
            f"found {facade_items}",
        )
else:
    fail(
        "scalar-script-facade-shape",
        "binary_object must contain exactly one private-parent scalar-script facade re-export",
    )

ordinary_facade_pattern = re.compile(
    r"(?m)^[ \t]*pub[ \t\n]*\([ \t\n]*super[ \t\n]*\)[ \t\n]+use"
    r"[ \t\n]+ordinary_leaf[ \t\n]*::[ \t\n]*\{(?P<body>[^{}]*)\}"
    r"[ \t\n]*;"
)
ordinary_facades = list(ordinary_facade_pattern.finditer(binary_root_code))
expected_ordinary_facade_names = {
    "DetachedAtomName",
    "DetachedPrimitive",
    "OrdinaryLeafApplyKind",
    "OrdinaryLeafBinaryOp",
    "OrdinaryLeafDraft",
    "OrdinaryLeafMetadataDraft",
    "OrdinaryLeafOp",
    "OrdinaryLeafPredicateOp",
    "OrdinaryLeafReadError",
    "OrdinaryLeafStackOp",
    "OrdinaryLeafUnaryOp",
    "RootFunctionConstantSelector",
    "decode_trusted_ordinary_leaf",
}
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
        fail(
            "ordinary-leaf-facade-shape",
            "binary_object must expose exactly the reviewed ordinary-leaf facade names; "
            f"found {ordinary_facade_items}",
        )
else:
    fail(
        "ordinary-leaf-facade-shape",
        "binary_object must contain exactly one private-parent ordinary-leaf facade re-export",
    )

facade_offsets = {
    match.start() for match in (*scalar_facades, *ordinary_facades)
}
for match in public_use_pattern.finditer(binary_root_code):
    if match.start() in facade_offsets:
        continue
    fail(
        "root-reexport",
        "binary_object root may re-export only the reviewed scalar-script and ordinary-leaf facades; found "
        + location(binary_root_relative, binary_root_source, match.start()),
    )

image_root_relative = "src/runtime/binary_object/bytecode_image/mod.rs"
image_root_source = read_source(image_root_relative)
image_root_code = rust_code_only(image_root_source)
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
    declarations = re.findall(
        rf"(?m)^[ \t]*mod[ \t]+{re.escape(module)}[ \t]*;[ \t]*$",
        image_root_code,
    )
    if len(declarations) != 1:
        fail(
            "image-private-module",
            f"{image_root_relative} must contain exactly one private `mod {module};` declaration",
        )
image_module_declarations = re.findall(
    r"(?m)^[ \t]*mod[ \t]+([A-Za-z_][A-Za-z0-9_]*)[ \t]*;[ \t]*$",
    image_root_code,
)
if sorted(image_module_declarations) != sorted(expected_image_modules):
    fail(
        "image-private-module",
        f"{image_root_relative} must contain only the reviewed private module set; "
        f"found {image_module_declarations}",
    )
for match in public_module_pattern.finditer(image_root_code):
    fail(
        "image-module-visibility",
        "bytecode_image submodules must remain private; found "
        + location(image_root_relative, image_root_source, match.start()),
    )

binary_root = root / "src/runtime/binary_object"
if binary_root.is_symlink() or not binary_root.is_dir():
    fail("missing-source", "src/runtime/binary_object must be a regular directory")
    binary_sources: list[Path] = []
else:
    binary_sources = sorted(binary_root.rglob("*.rs"))

# This scanner intentionally cross-checks the same codec identifiers in many
# independent invariants. Strip each source once: rust_code_only walks a file
# byte-by-byte, so recomputing it in every invariant makes the public gate
# quadratic in the number of checks and needlessly expensive for every canary.
binary_source_cache = {
    path: path.read_text(encoding="utf-8")
    for path in binary_sources
    if not path.is_symlink() and path.is_file()
}
binary_code_cache = {
    path: rust_code_only(source) for path, source in binary_source_cache.items()
}
expected_binary_visible_counts = {
    "src/runtime/binary_object/atoms.rs": 16,
    "src/runtime/binary_object/bytecode_image/atoms.rs": 11,
    "src/runtime/binary_object/bytecode_image/budget.rs": 32,
    "src/runtime/binary_object/bytecode_image/decode/function.rs": 8,
    "src/runtime/binary_object/bytecode_image/decode/mod.rs": 10,
    "src/runtime/binary_object/bytecode_image/decode/module.rs": 10,
    "src/runtime/binary_object/bytecode_image/encode/emit.rs": 2,
    "src/runtime/binary_object/bytecode_image/encode/mod.rs": 5,
    "src/runtime/binary_object/bytecode_image/encode/plan/function.rs": 1,
    "src/runtime/binary_object/bytecode_image/encode/plan/mod.rs": 10,
    "src/runtime/binary_object/bytecode_image/encode/plan/module.rs": 2,
    "src/runtime/binary_object/bytecode_image/mod.rs": 6,
    "src/runtime/binary_object/bytecode_image/model.rs": 111,
    "src/runtime/binary_object/bytecode_image/native_plan.rs": 27,
    "src/runtime/binary_object/code.rs": 27,
    "src/runtime/binary_object/function_envelope/mod.rs": 2,
    "src/runtime/binary_object/function_envelope/model.rs": 117,
    "src/runtime/binary_object/function_envelope/prefix.rs": 11,
    "src/runtime/binary_object/function_translate/capability.rs": 10,
    "src/runtime/binary_object/function_translate/dto.rs": 41,
    "src/runtime/binary_object/function_translate/mod.rs": 6,
    "src/runtime/binary_object/graph/arena.rs": 19,
    "src/runtime/binary_object/graph/decode.rs": 28,
    "src/runtime/binary_object/graph/encode.rs": 4,
    "src/runtime/binary_object/graph/mod.rs": 6,
    "src/runtime/binary_object/graph/model.rs": 56,
    "src/runtime/binary_object/graph/sab_transport.rs": 38,
    "src/runtime/binary_object/graph/write_state.rs": 21,
    "src/runtime/binary_object/mod.rs": 2,
    "src/runtime/binary_object/ordinary_leaf.rs": 30,
    "src/runtime/binary_object/pinned_atoms.rs": 9,
    "src/runtime/binary_object/pinned_opcodes.rs": 23,
    "src/runtime/binary_object/read_cursor.rs": 2,
    "src/runtime/binary_object/scalar_script.rs": 6,
    "src/runtime/binary_object/wire.rs": 46,
}
expected_fixture_visible_counts = {
    "src/runtime/binary_object/bytecode_image/atoms.rs": 1,
    "src/runtime/binary_object/bytecode_image/decode/mod.rs": 1,
    "src/runtime/binary_object/bytecode_image/mod.rs": 1,
    "src/runtime/binary_object/bytecode_image/model.rs": 2,
    "src/runtime/binary_object/bytecode_image/native_plan.rs": 27,
    "src/runtime/binary_object/graph/decode.rs": 1,
    "src/runtime/binary_object/graph/sab_transport.rs": 29,
    "src/runtime/binary_object/function_translate/capability.rs": 10,
    "src/runtime/binary_object/function_translate/dto.rs": 41,
    "src/runtime/binary_object/function_translate/mod.rs": 6,
    "src/runtime/binary_object/mod.rs": 2,
    "src/runtime/binary_object/ordinary_leaf.rs": 30,
    "src/runtime/binary_object/pinned_opcodes.rs": 23,
    "src/runtime/binary_object/read_cursor.rs": 2,
    "src/runtime/binary_object/scalar_script.rs": 6,
}
binary_visible_counts = {
    path.relative_to(root).as_posix(): len(
        re.findall(r"\bpub(?:[ \t\n]*\([^)]*\))?", code)
    )
    for path, code in binary_code_cache.items()
    if path.name != "tests.rs"
    and re.search(r"\bpub(?:[ \t\n]*\([^)]*\))?", code)
}
is_full_binary_inventory = binary_visible_counts == expected_binary_visible_counts
if binary_visible_counts not in (
    expected_binary_visible_counts,
    expected_fixture_visible_counts,
):
    fail(
        "binary-object-visible-surface",
        "binary_object production visibility counts drifted from the reviewed module surface; "
        f"found {binary_visible_counts}",
    )
bytecode_image_impl_header_pattern = re.compile(
    r"(?m)^[ \t]*impl\b(?P<header>[^{};]*)\{"
)
bytecode_image_impl_headers = {
    path.relative_to(root).as_posix(): [
        " ".join(match.group("header").split())
        for match in bytecode_image_impl_header_pattern.finditer(code)
    ]
    for path, code in binary_code_cache.items()
    if path.name != "tests.rs"
    and path.relative_to(root).as_posix().startswith(
        "src/runtime/binary_object/bytecode_image/"
    )
    and bytecode_image_impl_header_pattern.search(code)
}
expected_bytecode_image_impl_headers = {
    "src/runtime/binary_object/bytecode_image/atoms.rs": [
        "fmt::Display for ImageAtomError",
        "std::error::Error for ImageAtomError",
        "From<WireError> for ImageAtomError",
        "ImageAtomTable",
    ],
    "src/runtime/binary_object/bytecode_image/budget.rs": [
        "ModuleLimits",
        "fmt::Display for ModuleBudgetError",
        "std::error::Error for ModuleBudgetError",
        "BytecodeImageLimits",
        "fmt::Display for BytecodeImageBudgetError",
        "std::error::Error for BytecodeImageBudgetError",
        "ModuleUsage",
        "RemainingModuleBudget",
        "ModuleTotals",
        "FunctionUsage",
        "RemainingFunctionBudget",
        "FunctionTotals",
    ],
    "src/runtime/binary_object/bytecode_image/decode/function.rs": [
        "FunctionFrame",
        "FunctionTable",
    ],
    "src/runtime/binary_object/bytecode_image/decode/mod.rs": [
        "fmt::Display for BytecodeImageError",
        "std::error::Error for BytecodeImageError",
        "From<WireError> for BytecodeImageError",
        "From<ImageAtomError> for BytecodeImageError",
        "From<DecodeError<ImageOpaque>> for BytecodeImageError",
        "From<FunctionEnvelopeError> for BytecodeImageError",
        "From<ModuleBudgetError> for BytecodeImageError",
        "From<BytecodeImageBudgetError> for BytecodeImageError",
        "From<SabArchiveError> for BytecodeImageError",
        "AuthenticatedFunction",
        "AuthenticatedModule",
    ],
    "src/runtime/binary_object/bytecode_image/decode/module.rs": [
        "ModuleFrame",
        "ModuleTable",
    ],
    "src/runtime/binary_object/bytecode_image/encode/mod.rs": [
        "BytecodeImageEncodeOptions",
        "fmt::Display for BytecodeImageEncodeError",
        "std::error::Error for BytecodeImageEncodeError",
        "From<WireError> for BytecodeImageEncodeError",
        "From<GraphError> for BytecodeImageEncodeError",
        "From<DataWriteStateError> for BytecodeImageEncodeError",
        "From<BytecodeImageBudgetError> for BytecodeImageEncodeError",
        "From<ModuleBudgetError> for BytecodeImageEncodeError",
        "From<FunctionEnvelopeError> for BytecodeImageEncodeError",
        "From<CodeError> for BytecodeImageEncodeError",
    ],
    "src/runtime/binary_object/bytecode_image/encode/plan/function.rs": [
        "<'a> PlanBuilder<'a>",
    ],
    "src/runtime/binary_object/bytecode_image/encode/plan/mod.rs": [
        "<'a> PlanBuilder<'a>",
    ],
    "src/runtime/binary_object/bytecode_image/encode/plan/module.rs": [
        "<'a> PlanBuilder<'a>",
    ],
    "src/runtime/binary_object/bytecode_image/model.rs": [
        "FunctionId",
        "ModuleId",
        "ImageOpaque",
        "ImageValue",
        "ImageInstructionSpan",
        "ImageRelocation",
        "ImageCode",
        "ImageLocalVariable",
        "ImageClosureVariable",
        "ImageFunctionDebug",
        "ImageFunctionEnvelope",
        "FunctionRecord",
        "ModuleRequest",
        "ModuleExport",
        "ModuleImport",
        "ModuleRecord",
        "ImageAtomSummary",
        "BytecodeImage",
    ],
    "src/runtime/binary_object/bytecode_image/native_plan.rs": [
        "<'image> NativeAtomRef<'image>",
        "NativeLabel",
        "NativeOperands<'_>",
        "<'image> NativeInstruction<'image>",
        "<'image> NativeCodePlan<'image>",
        "NativePlanError",
        "fmt::Display for NativePlanError",
        "std::error::Error for NativePlanError",
    ],
}
expected_fixture_bytecode_image_impl_headers = {
    "src/runtime/binary_object/bytecode_image/model.rs": [
        "ImageLocalVariable",
        "ImageFunctionEnvelope",
        "BytecodeImage",
    ],
    "src/runtime/binary_object/bytecode_image/native_plan.rs": [
        "<'image> NativeAtomRef<'image>",
        "NativeLabel",
        "NativeOperands<'_>",
        "<'image> NativeInstruction<'image>",
        "<'image> NativeCodePlan<'image>",
        "NativePlanError",
        "fmt::Display for NativePlanError",
        "std::error::Error for NativePlanError",
    ],
}
if bytecode_image_impl_headers not in (
    expected_bytecode_image_impl_headers,
    expected_fixture_bytecode_image_impl_headers,
):
    fail(
        "bytecode-image-implementation-set",
        "bytecode_image implementation ownership drifted from the reviewed inherent and trait set; "
        f"found {bytecode_image_impl_headers}",
    )

native_plan_relative = "src/runtime/binary_object/bytecode_image/native_plan.rs"
native_plan_source = read_source(native_plan_relative)
native_plan_code = rust_code_only(native_plan_source)
native_plan_test_module_pattern = re.compile(
    r"(?m)^[ \t]*#[ \t\n]*\[[ \t\n]*cfg[ \t\n]*\([ \t\n]*test"
    r"[ \t\n]*\)[ \t\n]*\][ \t\n]*mod[ \t\n]+tests[ \t\n]*\{"
)
native_plan_test_module, native_plan_test_start, native_plan_test_end = unique_braced_item(
    native_plan_code,
    native_plan_test_module_pattern,
    "native-plan-test-module",
    "private cfg(test) native-plan test module",
)
if native_plan_test_module:
    native_plan_production_code = (
        native_plan_code[:native_plan_test_start]
        + blank(native_plan_code[native_plan_test_start:native_plan_test_end])
        + native_plan_code[native_plan_test_end:]
    )
    native_plan_production_source = (
        native_plan_source[:native_plan_test_start]
        + blank(native_plan_source[native_plan_test_start:native_plan_test_end])
        + native_plan_source[native_plan_test_end:]
    )
else:
    native_plan_production_code = native_plan_code
    native_plan_production_source = native_plan_source

native_plan_visibility = "pub(in crate::runtime::binary_object)"
native_plan_visibility_pattern = r"pub(?:[ \t\n]*\([^)]*\))?"
native_plan_visible_item_pattern = re.compile(
    rf"(?m)^[ \t]*(?P<visibility>{native_plan_visibility_pattern})[ \t\n]+"
    r"(?:(?:const|async|unsafe|extern)[ \t\n]+)*"
    r"(?P<kind>fn|enum|struct|trait|type|const|static|mod|use)\b"
    r"(?:[ \t\n]+(?P<name>[A-Za-z_][A-Za-z0-9_]*))?"
)
native_plan_visible_matches = list(
    native_plan_visible_item_pattern.finditer(native_plan_production_code)
)
native_plan_visible_items = [
    (match.group("kind"), match.group("name"))
    for match in native_plan_visible_matches
]
expected_native_plan_visible_items = [
    ("enum", "NativeAtomClass"),
    ("struct", "NativeAtomRef"),
    ("fn", "originates_from_input_atom_table"),
    ("fn", "class"),
    ("fn", "index"),
    ("fn", "manifest_string"),
    ("fn", "dynamic_string"),
    ("fn", "identity_description"),
    ("struct", "NativeLabel"),
    ("fn", "operand_pc"),
    ("fn", "displacement"),
    ("fn", "target_pc"),
    ("fn", "target_instruction"),
    ("enum", "NativeOperands"),
    ("fn", "format"),
    ("struct", "NativeInstruction"),
    ("fn", "byte_pc"),
    ("fn", "opcode"),
    ("fn", "operands"),
    ("struct", "NativeCodePlan"),
    ("fn", "function"),
    ("fn", "instructions"),
    ("fn", "native_pc_map"),
    ("fn", "instruction_at_native_pc"),
    ("enum", "NativePlanError"),
    ("fn", "is_label_target_error"),
    ("fn", "decode_native_code_plan"),
]
native_plan_visible_tokens = list(
    re.finditer(
        rf"(?<![A-Za-z0-9_]){native_plan_visibility_pattern}(?![A-Za-z0-9_])",
        native_plan_production_code,
    )
)
native_plan_visible_details = [
    (
        " ".join(match.group("visibility").split()),
        match.group("kind"),
        match.group("name"),
    )
    for match in native_plan_visible_matches
]
if (
    native_plan_visible_items != expected_native_plan_visible_items
    or len(native_plan_visible_matches) != len(native_plan_visible_tokens)
    or any(
        " ".join(match.group("visibility").split()) != native_plan_visibility
        for match in native_plan_visible_matches
    )
):
    fail(
        "native-plan-visible-surface",
        "the private native plan must expose only the reviewed binary_object-visible semantic DTO accessors; "
        f"found {native_plan_visible_details}",
    )

native_plan_use_pattern = re.compile(r"(?m)^[ \t]*use[ \t\n]+[^;]+;")
native_plan_uses = {
    " ".join(match.group(0).split())
    for match in native_plan_use_pattern.finditer(native_plan_production_code)
}
expected_native_plan_uses = {
    "use std::fmt;",
    "use super::{BytecodeImage, FunctionId, ImageAtom, ImageCode};",
    "use crate::runtime::binary_object::pinned_atoms::{FIRST_DYNAMIC_ATOM, PinnedAtomKind};",
    "use crate::runtime::binary_object::pinned_opcodes::{OpcodeFormat, PinnedOpcode};",
    "use crate::runtime::binary_object::wire::WireString;",
}
if native_plan_uses != expected_native_plan_uses:
    fail(
        "native-plan-dependency-set",
        "native_plan imports must remain the reviewed archive-only dependency set; "
        f"found {sorted(native_plan_uses)}",
    )

native_plan_all_type_pattern = re.compile(
    r"\b(?P<kind>enum|struct|union)[ \t\n]+"
    r"(?P<name>[A-Za-z_][A-Za-z0-9_]*)\b"
)
native_plan_all_type_items = [
    (match.group("kind"), match.group("name"))
    for match in native_plan_all_type_pattern.finditer(native_plan_production_code)
]
native_plan_type_keyword_count = len(
    re.findall(r"\b(?:enum|struct|union)\b", native_plan_production_code)
)
native_plan_type_pattern = re.compile(
    rf"(?m)^[ \t]*(?:{native_plan_visibility_pattern}[ \t\n]+)?"
    r"(?P<kind>enum|struct)[ \t\n]+(?P<name>[A-Za-z_][A-Za-z0-9_]*)"
    r"[^;{}]*\{"
)
native_plan_type_matches = list(native_plan_type_pattern.finditer(native_plan_production_code))
native_plan_type_items = [
    (match.group("kind"), match.group("name")) for match in native_plan_type_matches
]
expected_native_plan_type_items = [
    ("enum", "NativeAtomClass"),
    ("struct", "NativeAtomRef"),
    ("enum", "NativeAtomRefKind"),
    ("struct", "NativeLabel"),
    ("enum", "NativeOperands"),
    ("struct", "NativeInstruction"),
    ("struct", "NativeCodePlan"),
    ("enum", "NativePlanError"),
    ("struct", "DecodedCodePlan"),
]
if (
    native_plan_all_type_items != expected_native_plan_type_items
    or native_plan_type_items != expected_native_plan_type_items
    or native_plan_type_keyword_count != len(expected_native_plan_type_items)
):
    fail(
        "native-plan-type-set",
        "native_plan must contain only the reviewed braced semantic DTO and private decoder types; "
        f"found declarations {native_plan_all_type_items} and braced items {native_plan_type_items}",
    )

native_plan_function_names = re.findall(
    r"\bfn[ \t\n]+([A-Za-z_][A-Za-z0-9_]*)\b",
    native_plan_production_code,
)
native_plan_function_keyword_count = len(
    re.findall(r"\bfn\b", native_plan_production_code)
)
expected_native_plan_function_names = [
    "new",
    "originates_from_input_atom_table",
    "class",
    "index",
    "manifest_string",
    "dynamic_string",
    "identity_description",
    "operand_pc",
    "displacement",
    "target_pc",
    "target_instruction",
    "format",
    "byte_pc",
    "opcode",
    "operands",
    "function",
    "instructions",
    "native_pc_map",
    "instruction_at_native_pc",
    "is_label_target_error",
    "fmt",
    "decode_native_code_plan",
    "decode_code_plan",
    "validate_instruction_boundaries",
    "decode_operands",
    "implicit_integer",
    "implicit_slot",
    "invalid_implicit",
    "decode_label",
    "format_size",
    "read_u8",
    "read_i8",
    "read_u16",
    "read_i16",
    "read_u32",
    "read_i32",
    "read_array",
    "truncated_operand",
]
if (
    native_plan_function_names != expected_native_plan_function_names
    or native_plan_function_keyword_count != len(expected_native_plan_function_names)
):
    fail(
        "native-plan-function-set",
        "native_plan must contain only the reviewed constructors, accessors, and decoder helpers; "
        f"found {native_plan_function_names}",
    )

native_plan_non_function_consts = [
    name
    for name in re.findall(
        r"\bconst[ \t\n]+((?:r#)?[A-Za-z_][A-Za-z0-9_]*)\b",
        native_plan_production_code,
    )
    if name != "fn"
]
native_plan_static_items = re.findall(
    r"(?<!')\bstatic[ \t\n]+(?:mut[ \t\n]+)?"
    r"((?:r#)?[A-Za-z_][A-Za-z0-9_]*)\b",
    native_plan_production_code,
)
native_plan_const_keyword_count = len(
    re.findall(r"\bconst\b", native_plan_production_code)
)
native_plan_static_keyword_count = len(
    re.findall(r"\bstatic\b", native_plan_production_code)
)
if (
    native_plan_non_function_consts != ["WIDTH"]
    or native_plan_static_items
    or native_plan_const_keyword_count != 21
    or native_plan_static_keyword_count != 3
):
    fail(
        "native-plan-data-item-set",
        "native_plan must contain no const/static helper items beyond read_array's one const-generic width; "
        f"found const names {native_plan_non_function_consts} and static names {native_plan_static_items}",
    )

native_plan_stored_forbidden = re.compile(
    r"\b(?:ImageAtom|PinnedAtomId|BytecodeImage|ImageCode|ImageInstructionSpan|"
    r"ImageRelocation|Instruction|JsString|Value|Vm|VmHost|Runtime|Context|"
    r"RawValue|Heap|HeapObject|ObjectRef)\b|"
    r"(?:&[ \t\n]*(?:'[A-Za-z_][A-Za-z0-9_]*[ \t\n]+)?(?:mut[ \t\n]+)?)?"
    r"\[[ \t\n]*u8(?:[ \t\n]*;[^\]]+)?[ \t\n]*\]|"
    r"\b(?:Vec|Box|Arc|Rc|Cow)[ \t\n]*<[^>;{{}}]*\b"
    r"u8\b[^>;{{}}]*>"
)
for match in native_plan_type_matches:
    item_code, item_start, _ = braced_item_from_match(
        native_plan_production_code,
        match,
        "native-plan-facade-representation",
        f"{match.group('name')} type declaration",
    )
    forbidden = native_plan_stored_forbidden.search(item_code)
    if forbidden is not None:
        fail(
            "native-plan-facade-representation",
            "native-plan DTOs must not store raw image identities, native code bytes, or executable runtime representations; found "
            + location(
                native_plan_relative,
                native_plan_source,
                item_start + forbidden.start(),
            ),
        )

native_plan_visible_signature_forbidden = re.compile(
    r"\b(?:ImageAtom|PinnedAtomId|ImageCode|ImageInstructionSpan|ImageRelocation|"
    r"Instruction|JsString|Value|Vm|VmHost|Runtime|Context|RawValue|Heap|"
    r"HeapObject|ObjectRef)\b|"
    r"&[ \t\n]*(?:'[A-Za-z_][A-Za-z0-9_]*[ \t\n]+)?(?:mut[ \t\n]+)?"
    r"\[[ \t\n]*u8[ \t\n]*\]|"
    r"\b(?:Vec|Box|Arc|Rc|Cow)[ \t\n]*<[^>;{{}}]*\b"
    r"u8\b[^>;{{}}]*>"
)
for match in native_plan_visible_matches:
    if match.group("kind") != "fn":
        continue
    signature = native_plan_production_code[match.start():match.end()]
    opening_brace = native_plan_production_code.find("{", match.end())
    semicolon = native_plan_production_code.find(";", match.end())
    signature_end_candidates = [
        offset for offset in (opening_brace, semicolon) if offset >= 0
    ]
    if signature_end_candidates:
        signature = native_plan_production_code[match.start():min(signature_end_candidates)]
    forbidden = native_plan_visible_signature_forbidden.search(signature)
    if forbidden is not None:
        fail(
            "native-plan-visible-representation",
            "native-plan visible functions must not expose raw image identities, native code bytes, or executable runtime representations; found "
            + location(
                native_plan_relative,
                native_plan_source,
                match.start() + forbidden.start(),
            ),
        )

native_plan_runtime_dependency = re.compile(
    r"\bcrate[ \t\n]*::[ \t\n]*(?:bytecode|vm|heap|value)\b|"
    r"\b(?:Instruction|JsString|Value|Vm|VmHost|Runtime|Context|RuntimeError|"
    r"RawValue|Heap|HeapObject|ObjectRef)\b"
)
runtime_dependency = native_plan_runtime_dependency.search(native_plan_production_code)
if runtime_dependency is not None:
    fail(
        "native-plan-runtime-dependency",
        "native_plan must remain archive-only and independent of executable bytecode, VM, heap, and runtime String/Value representations; found "
        + location(
            native_plan_relative,
            native_plan_source,
            runtime_dependency.start(),
        ),
    )

native_plan_expansion_pattern = re.compile(
    r"\b(?:mod|trait|union|type)\b|"
    r"\b(?:include|include_bytes|include_str|macro_rules)[ \t\n]*!|"
    r"\bextern[ \t\n]+crate\b"
)
expansion = native_plan_expansion_pattern.search(native_plan_production_code)
if expansion is not None:
    fail(
        "native-plan-expansion",
        "native_plan must not hide additional modules, traits, aliases, unions, includes, or macro definitions; found "
        + location(native_plan_relative, native_plan_source, expansion.start()),
    )

native_atom_ref_pattern = re.compile(
    r"\bstruct[ \t\n]+NativeAtomRef[ \t\n]*<[ \t\n]*'image[ \t\n]*>"
    r"[ \t\n]*\{"
)
native_atom_ref_code, _, _ = unique_braced_item(
    native_plan_production_code,
    native_atom_ref_pattern,
    "native-plan-atom-projection",
    "sealed NativeAtomRef wrapper",
)
expected_native_atom_ref_source = """
    struct NativeAtomRef<'image> {
        kind: NativeAtomRefKind<'image>,
        from_input_atom_table: bool,
    }
"""
if (
    native_atom_ref_code
    and " ".join(native_atom_ref_code.split())
    != " ".join(expected_native_atom_ref_source.split())
):
    fail(
        "native-plan-atom-projection",
        "NativeAtomRef must remain a sealed wrapper around the sanitized private discriminator",
    )

native_atom_ref_kind_pattern = re.compile(
    r"\benum[ \t\n]+NativeAtomRefKind[ \t\n]*<[ \t\n]*'image[ \t\n]*>"
    r"[ \t\n]*\{"
)
native_atom_ref_kind_code, _, _ = unique_braced_item(
    native_plan_production_code,
    native_atom_ref_kind_pattern,
    "native-plan-atom-projection",
    "private sanitized NativeAtomRefKind discriminator",
)
expected_native_atom_ref_kind_source = """
    enum NativeAtomRefKind<'image> {
        Null,
        Index(u32),
        Manifest {
            class: NativeAtomClass,
            spelling: &'static str,
        },
        Dynamic(&'image WireString),
    }
"""
if (
    native_atom_ref_kind_code
    and " ".join(native_atom_ref_kind_code.split())
    != " ".join(expected_native_atom_ref_kind_source.split())
):
    fail(
        "native-plan-atom-projection",
        "NativeAtomRefKind must preserve only null, integer-index, manifest identity class/spelling, and image-borrowed dynamic String projections",
    )

def enum_variant_names(item_code: str) -> list[str]:
    opening = item_code.find("{")
    closing = item_code.rfind("}")
    if opening < 0 or closing <= opening:
        return []
    body = item_code[opening + 1:closing]
    variants: list[str] = []
    start = 0
    round_depth = 0
    square_depth = 0
    brace_depth = 0
    for offset, character in enumerate(body):
        if character == "(":
            round_depth += 1
        elif character == ")":
            round_depth -= 1
        elif character == "[":
            square_depth += 1
        elif character == "]":
            square_depth -= 1
        elif character == "{":
            brace_depth += 1
        elif character == "}":
            brace_depth -= 1
        elif character == "," and round_depth == square_depth == brace_depth == 0:
            segment = body[start:offset].strip()
            if segment:
                name = re.match(r"([A-Za-z_][A-Za-z0-9_]*)\b", segment)
                if name is None:
                    return []
                variants.append(name.group(1))
            start = offset + 1
    tail = body[start:].strip()
    if tail:
        name = re.match(r"([A-Za-z_][A-Za-z0-9_]*)\b", tail)
        if name is None:
            return []
        variants.append(name.group(1))
    return variants

native_atom_class_pattern = re.compile(
    rf"\b{native_plan_visibility_pattern}[ \t\n]+enum[ \t\n]+NativeAtomClass"
    r"[ \t\n]*\{"
)
native_atom_class_code, _, _ = unique_braced_item(
    native_plan_production_code,
    native_atom_class_pattern,
    "native-plan-atom-class",
    "NativeAtomClass enum",
)
native_atom_classes = enum_variant_names(native_atom_class_code)
if native_atom_classes != ["Null", "Index", "String", "Private", "Symbol"]:
    fail(
        "native-plan-atom-class",
        "NativeAtomClass must preserve null, integer-index, ordinary String, private-name, and Symbol identity classes; "
        f"found {native_atom_classes}",
    )

native_operands_pattern = re.compile(
    rf"\b{native_plan_visibility_pattern}[ \t\n]+enum[ \t\n]+NativeOperands"
    r"[ \t\n]*<[ \t\n]*'image[ \t\n]*>[ \t\n]*\{"
)
native_operands_code, _, _ = unique_braced_item(
    native_plan_production_code,
    native_operands_pattern,
    "native-plan-operand-formats",
    "NativeOperands enum",
)
native_operand_variants = enum_variant_names(native_operands_code)

if is_full_binary_inventory:
    pinned_opcode_relative = "src/runtime/binary_object/pinned_opcodes.rs"
    pinned_opcode_code = binary_code_cache[root / pinned_opcode_relative]
    opcode_format_pattern = re.compile(
        rf"\b{native_plan_visibility_pattern.replace('binary_object', 'runtime')}"
        r"[ \t\n]+enum[ \t\n]+OpcodeFormat[ \t\n]*\{"
    )
    opcode_format_code, _, _ = unique_braced_item(
        pinned_opcode_code,
        opcode_format_pattern,
        "native-plan-operand-formats",
        "pinned OpcodeFormat enum",
    )
    opcode_format_variants = enum_variant_names(opcode_format_code)
    if native_operand_variants != opcode_format_variants:
        fail(
            "native-plan-operand-formats",
            "NativeOperands must remain a one-for-one, ordered projection of the pinned OpcodeFormat table; "
            f"found native {native_operand_variants} versus pinned {opcode_format_variants}",
        )

native_format_method_pattern = re.compile(
    rf"\b{native_plan_visibility_pattern}[ \t\n]+const[ \t\n]+fn"
    r"[ \t\n]+format[ \t\n]*\([ \t\n]*&self[ \t\n]*\)"
    r"[ \t\n]*->[ \t\n]*OpcodeFormat[ \t\n]*\{"
)
native_format_method_code, _, _ = unique_braced_item(
    native_plan_production_code,
    native_format_method_pattern,
    "native-plan-operand-formats",
    "NativeOperands::format mapping",
)
native_format_pairs = re.findall(
    r"Self[ \t\n]*::[ \t\n]*([A-Za-z_][A-Za-z0-9_]*)"
    r"[ \t\n]*(?:\([^=]*?\)|\{[^=]*?\})?[ \t\n]*=>"
    r"[ \t\n]*OpcodeFormat[ \t\n]*::[ \t\n]*([A-Za-z_][A-Za-z0-9_]*)",
    native_format_method_code,
)
if (
    [native for native, _ in native_format_pairs] != native_operand_variants
    or any(native != opcode_format for native, opcode_format in native_format_pairs)
):
    fail(
        "native-plan-operand-formats",
        "NativeOperands::format must map every typed operand variant to its identically named pinned format exactly once; "
        f"found {native_format_pairs}",
    )

native_plan_semantic_seals = [
    (
        "atom projection and accessors",
        re.compile(
            r"\bimpl[ \t\n]*<[ \t\n]*'image[ \t\n]*>[ \t\n]+"
            r"NativeAtomRef[ \t\n]*<[ \t\n]*'image[ \t\n]*>[ \t\n]*\{"
        ),
        "be6f900535b456104e234a5babec4c1f1dae2d440c7fb25129724fe30dd6529a",
    ),
    (
        "label representation",
        re.compile(
            rf"\b{native_plan_visibility_pattern}[ \t\n]+struct"
            r"[ \t\n]+NativeLabel[ \t\n]*\{"
        ),
        "f6dfbd46ce01e82b9ac4d91be84f5027ea5da91e97d15a7d164a9a9c98f97731",
    ),
    (
        "label accessors",
        re.compile(r"\bimpl[ \t\n]+NativeLabel[ \t\n]*\{"),
        "ad6e4d0ac76e7a0b242641e2adf87b8a279de581bf168b0617ac4ead7105db9b",
    ),
    (
        "typed operand representation",
        re.compile(
            rf"\b{native_plan_visibility_pattern}[ \t\n]+enum"
            r"[ \t\n]+NativeOperands[^{{;]*\{"
        ),
        "77fb7a85d1c0210a34b668ea96fa9e7244471cbe122354de21faebc6198c1f06",
    ),
    (
        "instruction representation",
        re.compile(
            rf"\b{native_plan_visibility_pattern}[ \t\n]+struct"
            r"[ \t\n]+NativeInstruction[^{{;]*\{"
        ),
        "bda6ce3d26ef21cb5ca863fae40c591859ee811dbe2eb38f7c3991b8bd5f21fc",
    ),
    (
        "instruction accessors",
        re.compile(
            r"\bimpl[ \t\n]*<[ \t\n]*'image[ \t\n]*>[ \t\n]+"
            r"NativeInstruction[^{{;]*\{"
        ),
        "d2e377ac2764175ade3acae2d3b460d8cbd64a6bd7d56a29b7d009e3a91b15b1",
    ),
    (
        "code-plan representation",
        re.compile(
            rf"\b{native_plan_visibility_pattern}[ \t\n]+struct"
            r"[ \t\n]+NativeCodePlan[^{{;]*\{"
        ),
        "1b44645bb4a3b329a0e7fb19564ea0b69c52dc4694fbe208454fca3eaa0ce055",
    ),
    (
        "code-plan accessors",
        re.compile(
            r"\bimpl[ \t\n]*<[ \t\n]*'image[ \t\n]*>[ \t\n]+"
            r"NativeCodePlan[^{{;]*\{"
        ),
        "8689b46353fc3296ca813b31a399017c398dae18d8ba226e7f5c901d72424e08",
    ),
    (
        "error representation",
        re.compile(
            rf"\b{native_plan_visibility_pattern}[ \t\n]+enum"
            r"[ \t\n]+NativePlanError[ \t\n]*\{"
        ),
        "9d79a33a085e1fc0f2e71bc60430fb15f870449847182d2affbd3b8fea2328e6",
    ),
    (
        "label-target error classification",
        re.compile(r"\bimpl[ \t\n]+NativePlanError[ \t\n]*\{"),
        "0610bf345f915003023c37f91cff4250885bf53cff7da15ca64486ecc8c50817",
    ),
    (
        "authenticated entrypoint",
        re.compile(
            rf"\b{native_plan_visibility_pattern}[ \t\n]+fn"
            r"[ \t\n]+decode_native_code_plan[^{{;]*\{"
        ),
        "df01f2c531c6981903a7d765fe9d9e6b5e9367d5970f17fff0938b2f907f0376",
    ),
    (
        "code and relocation decoder",
        re.compile(r"\bfn[ \t\n]+decode_code_plan[^{{;]*\{"),
        "2a1fa56db24fc98351ba5035403576b36539cc560b8ebbf5bbda98c00333171e",
    ),
    (
        "instruction-boundary authentication",
        re.compile(r"\bfn[ \t\n]+validate_instruction_boundaries[^{{;]*\{"),
        "4f33d2a1edef051800bf810be6064771d33804b98287bc5b075e3eec514babde",
    ),
    (
        "operand decoder",
        re.compile(r"\bfn[ \t\n]+decode_operands[^{{;]*\{"),
        "3bd2e04c1e7c2d422b9543ba356c5a627600fa23dc45f40c6de923001e2ee393",
    ),
    (
        "implicit integer decoder",
        re.compile(r"\bfn[ \t\n]+implicit_integer[^{{;]*\{"),
        "d5880d502e8c08f31320fc823eacaf65bbc71ef00376385f6235053bdc8bf379",
    ),
    (
        "implicit slot decoder",
        re.compile(r"\bfn[ \t\n]+implicit_slot[^{{;]*\{"),
        "a5aa9080cfe562279addb030926ab547d976c126594f66ccbf2a0496e59991b4",
    ),
    (
        "label decoder",
        re.compile(r"\bfn[ \t\n]+decode_label[^{{;]*\{"),
        "535ab4c806e3fd2536e7bdd2999fabf2766f0f05ed6587a311e570d27ffc8d73",
    ),
    (
        "operand format sizes",
        re.compile(r"\bconst[ \t\n]+fn[ \t\n]+format_size[^{{;]*\{"),
        "7124a7e74681fded01f1f6018132c8fb25e2e4bc76b67b86f3df045b8116dde6",
    ),
]
for description, pattern, expected_hash in native_plan_semantic_seals:
    item_code, _, _ = unique_braced_item(
        native_plan_production_code,
        pattern,
        "native-plan-semantic-seal",
        description,
    )
    if item_code and normalized_code_sha256(item_code) != expected_hash:
        fail(
            "native-plan-semantic-seal",
            f"native_plan {description} drifted from its reviewed normalized implementation",
        )

native_plan_implicit_string_patterns = {
    "push_minus1": re.compile(
        r"opcode[ \t\n]*\.[ \t\n]*name[ \t\n]*\([ \t\n]*\)"
        r"[ \t\n]*==[ \t\n]*\"push_minus1\""
    ),
    "push_ prefix": re.compile(
        r"opcode[ \t\n]*\.[ \t\n]*name[ \t\n]*\([ \t\n]*\)"
        r"[ \t\n]*\.[ \t\n]*strip_prefix[ \t\n]*\([ \t\n]*\"push_\""
        r"[ \t\n]*\)"
    ),
    "local short forms": re.compile(
        r"&[ \t\n]*\[[ \t\n]*\"get_loc\"[ \t\n]*,"
        r"[ \t\n]*\"put_loc\"[ \t\n]*,[ \t\n]*\"set_loc\"[ \t\n]*\]"
    ),
    "argument short forms": re.compile(
        r"&[ \t\n]*\[[ \t\n]*\"get_arg\"[ \t\n]*,"
        r"[ \t\n]*\"put_arg\"[ \t\n]*,[ \t\n]*\"set_arg\"[ \t\n]*\]"
    ),
    "variable-reference short forms": re.compile(
        r"&[ \t\n]*\[[ \t\n]*\"get_var_ref\"[ \t\n]*,"
        r"[ \t\n]*\"put_var_ref\"[ \t\n]*,[ \t\n]*\"set_var_ref\""
        r"[ \t\n]*\]"
    ),
    "call short form": re.compile(
        r"OpcodeFormat[ \t\n]*::[ \t\n]*NPopX[^=]*=>[^;]*"
        r"&[ \t\n]*\[[ \t\n]*\"call\"[ \t\n]*\]",
        re.DOTALL,
    ),
}
for description, pattern in native_plan_implicit_string_patterns.items():
    if len(pattern.findall(native_plan_production_source)) != 1:
        fail(
            "native-plan-implicit-opcode-set",
            f"native_plan must retain exactly one reviewed {description} implicit-opcode spelling",
        )

native_plan_declaration_pattern = re.compile(
    r"(?m)^[ \t]*mod[ \t]+native_plan[ \t]*;[ \t]*$"
)
native_plan_facade_pattern = re.compile(
    rf"\b{native_plan_visibility_pattern}[ \t\n]+use[ \t\n]+native_plan"
    r"[ \t\n]*::[ \t\n]*\{[ \t\n]*NativeAtomClass[ \t\n]*,"
    r"[ \t\n]*NativeAtomRef[ \t\n]*,[ \t\n]*NativeCodePlan[ \t\n]*,"
    r"[ \t\n]*NativeOperands[ \t\n]*,[ \t\n]*decode_native_code_plan"
    r"[ \t\n]*,?[ \t\n]*\}[ \t\n]*;"
)
native_plan_declarations = list(native_plan_declaration_pattern.finditer(image_root_code))
native_plan_facades = list(native_plan_facade_pattern.finditer(image_root_code))
if len(native_plan_declarations) != 1 or len(native_plan_facades) != 1:
    fail(
        "native-plan-private-stage",
        "native_plan must remain one private bytecode_image child with one exact binary_object-only semantic facade",
    )

for path, code in binary_code_cache.items():
    relative = path.relative_to(root).as_posix()
    if relative == native_plan_relative:
        continue
    mentions = list(re.finditer(r"\bnative_plan\b", code))
    if relative == image_root_relative:
        allowed_ranges = [
            (match.start(), match.end())
            for match in (*native_plan_declarations, *native_plan_facades)
        ]
        unexpected = [
            mention
            for mention in mentions
            if not any(start <= mention.start() < end for start, end in allowed_ranges)
        ]
        if unexpected:
            fail(
                "native-plan-private-stage",
                "native_plan module and facade shape drifted; found "
                + ", ".join(
                    location(relative, binary_source_cache[path], mention.start())
                    for mention in unexpected
                ),
            )
    elif relative == "src/runtime/binary_object/function_translate/mod.rs":
        continue
    elif mentions:
        fail(
            "native-plan-private-stage",
            "native_plan may be named only by its private module/facade and the reviewed scalar/ordinary consumers; found "
            + ", ".join(
                location(relative, binary_source_cache[path], mention.start())
                for mention in mentions
            ),
        )

native_plan_facade_symbols = (
    "NativeAtomClass",
    "NativeAtomRef",
    "NativeCodePlan",
    "NativeOperands",
    "decode_native_code_plan",
)
allowed_native_plan_symbol_files = {
    native_plan_relative,
    image_root_relative,
    "src/runtime/binary_object/function_translate/mod.rs",
}
for path, code in binary_code_cache.items():
    relative = path.relative_to(root).as_posix()
    if path.name == "tests.rs" or relative in allowed_native_plan_symbol_files:
        continue
    for symbol in native_plan_facade_symbols:
        mention = re.search(rf"\b{symbol}\b", code)
        if mention is not None:
            fail(
                "native-plan-consumer-set",
                "only function_translate may consume the reviewed native-plan facade; found "
                + location(relative, binary_source_cache[path], mention.start()),
            )

bytecode_image_alias_pattern = re.compile(
    r"\btype[ \t\n]+[A-Za-z_][A-Za-z0-9_]*(?:[ \t\n]*<[^;=]*>)?"
    r"[ \t\n]*=[^;]*\bBytecodeImage\b"
    r"|\buse\b[^;]*\bBytecodeImage[ \t\n]+as[ \t\n]+"
    r"[A-Za-z_][A-Za-z0-9_]*"
)
for path, code in binary_code_cache.items():
    relative = path.relative_to(root).as_posix()
    if not relative.startswith("src/runtime/binary_object/bytecode_image/"):
        continue
    for match in bytecode_image_alias_pattern.finditer(code):
        fail(
            "bytecode-image-alias",
            "BytecodeImage must not acquire a type or import alias that can hide implementation ownership; found "
            + location(relative, binary_source_cache[path], match.start()),
        )
for path, code in binary_code_cache.items():
    for match in re.finditer(
        r"(?<![A-Za-z0-9_])(?:r#)?include[ \t\n]*!",
        code,
    ):
        relative = path.relative_to(root).as_posix()
        fail(
            "forbidden-source-include",
            "binary_object production sources must not splice unscanned Rust source; found "
            + location(relative, binary_source_cache[path], match.start()),
        )

function_translate_root = "src/runtime/binary_object/function_translate"
function_translate_relative = f"{function_translate_root}/mod.rs"
function_translate_capability_relative = f"{function_translate_root}/capability.rs"
function_translate_dto_relative = f"{function_translate_root}/dto.rs"
expected_function_translate_sources = {
    function_translate_relative,
    function_translate_capability_relative,
    function_translate_dto_relative,
}
found_function_translate_sources = {
    path.relative_to(root).as_posix()
    for path in binary_sources
    if path.relative_to(root).as_posix().startswith(f"{function_translate_root}/")
}
if found_function_translate_sources != expected_function_translate_sources:
    fail(
        "function-translate-module-set",
        "function_translate must contain only the reviewed module root, capability registry, and sanitized DTO; "
        f"found {sorted(found_function_translate_sources)}",
    )

function_translate_source = read_source(function_translate_relative)
function_translate_code = rust_code_only(function_translate_source)
function_translate_production_code = function_translate_code.split("#[cfg(test)]", 1)[0]
function_translate_production_source = function_translate_source.split("#[cfg(test)]", 1)[0]
function_translate_modules = re.findall(
    r"(?m)^[ \t]*mod[ \t]+([A-Za-z_][A-Za-z0-9_]*)[ \t]*;[ \t]*$",
    function_translate_production_code,
)
if function_translate_modules != ["capability", "dto"]:
    fail(
        "function-translate-module-set",
        "function_translate must retain exactly the private capability and dto children; "
        f"found {function_translate_modules}",
    )
for match in public_module_pattern.finditer(function_translate_production_code):
    fail(
        "function-translate-module-visibility",
        "function_translate children must remain private; found "
        + location(function_translate_relative, function_translate_source, match.start()),
    )

capability_source = read_source(function_translate_capability_relative)
capability_production_source = capability_source.split("#[cfg(test)]", 1)[0]
capability_production_code = rust_code_only(capability_production_source)
dto_source = read_source(function_translate_dto_relative)
dto_production_source = dto_source.split("#[cfg(test)]", 1)[0]
dto_production_code = rust_code_only(dto_production_source)

recipe_code, _, _ = unique_braced_item(
    capability_production_code,
    re.compile(r"\benum[ \t\n]+Recipe[ \t\n]*\{"),
    "function-translate-recipe-shape",
    "Recipe enum",
)
expected_recipe_variants = """
    Nop Object ToObject PushThis PushI32 PushConstant PushAtom PushUndefined PushNull PushFalse PushTrue
    PushBigIntI32 PushEmptyString Stack Unary PostDec PostInc GetLocal PutLocal
    SetLocal GetArgument PutArgument SetArgument Binary Predicate IfFalse IfTrue
    Goto Call TailCall Construct CallMethod TailCallMethod ArrayFrom Apply Return
    ReturnUndefined Throw ThrowReadOnly
""".split()
counted_invocation_variant_names = (
    "Call", "TailCall", "Construct", "CallMethod", "TailCallMethod", "ArrayFrom"
)
invocation_variant_names = (*counted_invocation_variant_names, "Apply")
unit_recipe_variant_names = ("Nop", "Object", "ToObject", "PushThis")
unit_recipe_shapes = {
    name: (
        len(re.findall(rf"\b{name}[ \t\n]*,", recipe_code)),
        bool(re.search(rf"\b{name}[ \t\n]*[({{]", recipe_code)),
    )
    for name in unit_recipe_variant_names
}
recipe_invocation_shapes = {
    name: (
        len(re.findall(rf"\b{name}[ \t\n]*,", recipe_code)),
        bool(re.search(rf"\b{name}[ \t\n]*[({{]", recipe_code)),
    )
    for name in invocation_variant_names
}
if (
    enum_variant_names(recipe_code) != expected_recipe_variants
    or any(unit_recipe_shapes[name] != (1, False) for name in unit_recipe_variant_names)
    or any(
        recipe_invocation_shapes[name] != (1, False)
        for name in invocation_variant_names
    )
):
    fail(
        "function-translate-recipe-shape",
        "Recipe must retain the exact reviewed inventory with unit Nop/Object/ToObject/PushThis recipes and explicit terminal completions; "
        f"found {enum_variant_names(recipe_code)} with unit shapes {unit_recipe_shapes} "
        f"and invocation shapes {recipe_invocation_shapes}",
    )

dto_function_op_code, _, _ = unique_braced_item(
    dto_production_code,
    re.compile(
        r"\benum[ \t\n]+FunctionOp[ \t\n]*<[ \t\n]*'image[ \t\n]*>[ \t\n]*\{"
    ),
    "function-translate-dto-shape",
    "FunctionOp enum",
)
expected_function_op_variants = """
    Blocked OutsideTarget Nop Object ToObject PushThis PushI32 PushConstant PushAtom PushUndefined PushNull
    PushBool PushBigIntI32 PushEmptyString Stack Unary PostDec PostInc GetLocal
    PutLocal SetLocal GetArgument PutArgument SetArgument Binary Predicate IfFalse
    IfTrue Goto Call TailCall Construct CallMethod TailCallMethod ArrayFrom Apply
    Return ReturnUndefined Throw ThrowReadOnly
""".split()
function_invocation_payloads = {
    name: [
        " ".join(payload.split())
        for payload in re.findall(
            rf"\b{name}[ \t\n]*\(([^()]*)\)[ \t\n]*,", dto_function_op_code
        )
    ]
    for name in invocation_variant_names
}
if (
    enum_variant_names(dto_function_op_code) != expected_function_op_variants
    or any(function_invocation_payloads[name] != ["u16"]
           for name in counted_invocation_variant_names)
    or function_invocation_payloads["Apply"] != ["FunctionApplyKind"]
):
    fail(
        "function-translate-dto-shape",
        "FunctionOp must retain the exact reviewed inventory with operand-free Nop/Object/ToObject/PushThis, distinct counted u16 invocation payloads, a typed Apply kind, and an operand-free Throw completion; "
        f"found {enum_variant_names(dto_function_op_code)} with invocation payloads {function_invocation_payloads}",
    )
function_throw_read_only_payloads = [
    " ".join(payload.split())
    for payload in re.findall(
        r"\bThrowReadOnly[ \t\n]*\(([^()]*)\)[ \t\n]*,",
        dto_function_op_code,
    )
]
if function_throw_read_only_payloads != ["AtomOperand<'image>"]:
    fail(
        "function-translate-dto-shape",
        "FunctionOp::ThrowReadOnly must retain exactly one borrowed sanitized AtomOperand payload",
    )

dto_apply_kind_code, _, _ = unique_braced_item(
    dto_production_code,
    re.compile(
        r"\bpub[ \t\n]*\([ \t\n]*in[ \t\n]+crate[ \t\n]*::[ \t\n]*runtime"
        r"[ \t\n]*::[ \t\n]*binary_object[ \t\n]*\)[ \t\n]+enum"
        r"[ \t\n]+FunctionApplyKind[ \t\n]*\{"
    ),
    "function-translate-apply-kind",
    "typed sanitized apply kind",
)
if enum_variant_names(dto_apply_kind_code) != ["Call", "Construct"]:
    fail(
        "function-translate-apply-kind",
        "FunctionApplyKind must expose only canonical call and construct semantics; "
        f"found {enum_variant_names(dto_apply_kind_code)}",
    )

pinned_opcode_relative = "src/runtime/binary_object/pinned_opcodes.rs"
pinned_opcode_source = read_source(pinned_opcode_relative)
pinned_opcode_production_source = pinned_opcode_source.split("#[cfg(test)]", 1)[0]
pinned_descriptor_pattern = re.compile(
    r"PinnedOpcodeInfo[ \t\n]*::[ \t\n]*new[ \t\n]*\("
    r"[ \t\n]*\"([^\"]+)\"[ \t\n]*,[ \t\n]*(\d+)"
    r"[ \t\n]*,[ \t\n]*(\d+)[ \t\n]*,[ \t\n]*(\d+)"
    r"[ \t\n]*,[ \t\n]*OpcodeFormat[ \t\n]*::[ \t\n]*"
    r"([A-Za-z_][A-Za-z0-9_]*)[ \t\n]*\)"
)
pinned_descriptors = [
    (name, int(size), int(n_pop), int(n_push), operand_format)
    for name, size, n_pop, n_push, operand_format in pinned_descriptor_pattern.findall(
        pinned_opcode_production_source
    )
]
if len(pinned_descriptors) != 244:
    fail(
        "function-translate-registry-descriptor",
        "the pinned descriptor table must retain exactly 244 ordered entries; "
        f"found {len(pinned_descriptors)}",
    )

registry_row_pattern = re.compile(
    r"(?m)^[ \t]*row![ \t]*\([ \t]*(\d+)[ \t]*,[ \t]*"
    r"([A-Za-z_][A-Za-z0-9_]*)[ \t]*,[ \t]*"
    r"(Blocked|ScalarOnly|OrdinaryOnly|Shared)[ \t]*,[ \t]*"
    r"([^,\n]+)[ \t]*\)[ \t]*,[ \t]*$"
)
registry_rows = [
    (int(raw), operand_format, audience, " ".join(detail.split()))
    for raw, operand_format, audience, detail in registry_row_pattern.findall(
        capability_production_source
    )
]
registry_raws = [raw for raw, _, _, _ in registry_rows]
if registry_raws != list(range(244)):
    fail(
        "function-translate-registry-raw",
        "the capability registry must contain one contiguous raw-indexed row for each opcode 0 through 243; "
        f"found {registry_raws}",
    )
if len(pinned_descriptors) == 244:
    format_mismatches = [
        (raw, operand_format, pinned_descriptors[raw][4])
        for raw, operand_format, _, _ in registry_rows
        if raw >= len(pinned_descriptors)
        or operand_format != pinned_descriptors[raw][4]
    ]
    if format_mismatches:
        fail(
            "function-translate-registry-descriptor",
            "each raw-indexed capability row must retain the operand format of its pinned descriptor; "
            f"found {format_mismatches}",
        )

registry_audience_counts = {
    audience: sum(row_audience == audience for _, _, row_audience, _ in registry_rows)
    for audience in ("Blocked", "ScalarOnly", "OrdinaryOnly", "Shared")
}
expected_registry_audience_counts = dict(zip(
    ("Blocked", "ScalarOnly", "OrdinaryOnly", "Shared"), (111, 1, 103, 29)
))
derived_registry_counts = (
    registry_audience_counts["ScalarOnly"] + registry_audience_counts["Shared"],
    registry_audience_counts["OrdinaryOnly"] + registry_audience_counts["Shared"],
    244 - registry_audience_counts["Blocked"],
)
if (
    registry_audience_counts != expected_registry_audience_counts
    or derived_registry_counts != (30, 132, 133)
):
    fail(
        "function-translate-registry-audience",
        "the centralized registry must preserve the exact final stage-three-I physical cohorts; "
        f"found {registry_audience_counts} with scalar/ordinary/union {derived_registry_counts}",
    )

expected_admitted_registry: dict[int, tuple[str, str]] = {}


def expect_admitted(audience: str, recipe: str, raws: tuple[int, ...]) -> None:
    for raw in raws:
        if raw in expected_admitted_registry:
            fail(
                "function-translate-registry-policy",
                f"internal gate expectation names admitted raw opcode {raw} twice",
            )
        expected_admitted_registry[raw] = (audience, recipe)


expect_admitted("Shared", "Recipe::PushI32", (1, *range(178, 189)))
expect_admitted("Shared", "Recipe::PushConstant", (2, 189))
expect_admitted("Shared", "Recipe::PushUndefined", (6,))
expect_admitted("Shared", "Recipe::PushNull", (7,))
expect_admitted("Shared", "Recipe::PushFalse", (9,))
expect_admitted("Shared", "Recipe::PushTrue", (10,))
expect_admitted("Shared", "Recipe::Return", (40,))
for raw, operation in zip((138, 139, 140, 141, 147, 148, 149),
                          ("Neg", "Plus", "Dec", "Inc", "BitNot", "LogicalNot", "TypeOf")):
    expect_admitted("Shared", f"Recipe::Unary(FunctionUnaryOp::{operation})", (raw,))
expect_admitted("Shared", "Recipe::PushBigIntI32", (176,))
expect_admitted("Shared", "Recipe::PushEmptyString", (191,))
expect_admitted("Shared", "Recipe::SetLocal", (203,))
expect_admitted("ScalarOnly", "Recipe::PushAtom", (4,))
for raw, recipe in zip(
    range(14, 33),
    (
        "Direct(FunctionStackOp::Drop)", "Direct(FunctionStackOp::Nip)", "Nip1",
        "Direct(FunctionStackOp::Dup)", "Direct(FunctionStackOp::Dup1)", "Dup2",
        "Direct(FunctionStackOp::Dup3)", "Direct(FunctionStackOp::Insert2)",
        "Direct(FunctionStackOp::Insert3)", "Direct(FunctionStackOp::Insert4)",
        "Direct(FunctionStackOp::Perm3)", "Direct(FunctionStackOp::Perm4)",
        "Direct(FunctionStackOp::Perm5)", "Direct(FunctionStackOp::Swap)", "Swap2",
        "Rot3Left", "Rot3Right", "Direct(FunctionStackOp::Rot4Left)", "Rot5Left",
    ),
):
    expect_admitted("OrdinaryOnly", f"Recipe::Stack(StackRecipe::{recipe})", (raw,))
expect_admitted("OrdinaryOnly", "Recipe::ReturnUndefined", (41,))
expect_admitted("OrdinaryOnly", "Recipe::GetLocal", (85, 192, 195, 196, 197, 198))
expect_admitted("OrdinaryOnly", "Recipe::PutLocal", (86, 193, 199, 200, 201, 202))
expect_admitted("OrdinaryOnly", "Recipe::SetLocal", (87, 194, 204, 205, 206))
expect_admitted("OrdinaryOnly", "Recipe::GetArgument", (88, 207, 208, 209, 210))
expect_admitted("OrdinaryOnly", "Recipe::PutArgument", (89, 211, 212, 213, 214))
expect_admitted("OrdinaryOnly", "Recipe::SetArgument", (90, 215, 216, 217, 218))
expect_admitted("OrdinaryOnly", "Recipe::IfFalse", (104, 232))
expect_admitted("OrdinaryOnly", "Recipe::IfTrue", (105, 233))
expect_admitted("OrdinaryOnly", "Recipe::Goto", (106, 234, 235))
expect_admitted("OrdinaryOnly", "Recipe::Call", (34, 236, 237, 238, 239))
expect_admitted("OrdinaryOnly", "Recipe::TailCall", (35,))
expect_admitted("OrdinaryOnly", "Recipe::Construct", (33,))
expect_admitted("OrdinaryOnly", "Recipe::CallMethod", (36,))
expect_admitted("OrdinaryOnly", "Recipe::TailCallMethod", (37,))
expect_admitted("OrdinaryOnly", "Recipe::ArrayFrom", (38,))
expect_admitted("OrdinaryOnly", "Recipe::Apply", (39,))
expect_admitted("OrdinaryOnly", "Recipe::Throw", (48,))
expect_admitted("OrdinaryOnly", "Recipe::ThrowReadOnly", (49,))
expect_admitted("OrdinaryOnly", "Recipe::Nop", (177,))
expect_admitted("OrdinaryOnly", "Recipe::Object", (11,))
expect_admitted("OrdinaryOnly", "Recipe::ToObject", (111,))
expect_admitted("OrdinaryOnly", "Recipe::PushThis", (8,))
expect_admitted("OrdinaryOnly", "Recipe::PostDec", (142,))
expect_admitted("OrdinaryOnly", "Recipe::PostInc", (143,))
for raw, operation in (
    (152, "Mul"), (153, "Div"), (154, "Mod"), (155, "Add"), (156, "Sub"),
    (157, "Pow"), (158, "Shl"), (159, "Sar"), (160, "Shr"),
    (161, "LessThan"), (162, "LessThanOrEqual"), (163, "GreaterThan"),
    (164, "GreaterThanOrEqual"), (167, "Equal"), (168, "NotEqual"),
    (169, "StrictEqual"), (170, "StrictNotEqual"), (171, "BitAnd"),
    (172, "BitXor"), (173, "BitOr"),
):
    expect_admitted("OrdinaryOnly", f"Recipe::Binary(FunctionBinaryOp::{operation})", (raw,))
for raw, predicate in (
    (174, "IsUndefinedOrNull"), (240, "IsUndefined"), (241, "IsNull"),
    (242, "TypeOfIsUndefined"), (243, "TypeOfIsFunction"),
):
    expect_admitted("OrdinaryOnly", f"Recipe::Predicate(FunctionPredicateOp::{predicate})", (raw,))
found_admitted_registry = {
    raw: (audience, detail)
    for raw, _, audience, detail in registry_rows
    if audience != "Blocked"
}
if found_admitted_registry != expected_admitted_registry:
    fail(
        "function-translate-registry-policy",
        "the registry must preserve every admitted raw opcode, audience, and semantic recipe; "
        f"found {found_admitted_registry}",
    )

expected_stage_boundaries = {
    8: (("push_this", 1, 0, 1, "None"), "OrdinaryOnly", "Recipe::PushThis"),
    11: (("object", 1, 0, 1, "None"), "OrdinaryOnly", "Recipe::Object"),
    33: (("call_constructor", 3, 2, 1, "NPop"), "OrdinaryOnly", "Recipe::Construct"),
    34: (("call", 3, 1, 1, "NPop"), "OrdinaryOnly", "Recipe::Call"),
    35: (("tail_call", 3, 1, 0, "NPop"), "OrdinaryOnly", "Recipe::TailCall"),
    36: (("call_method", 3, 2, 1, "NPop"), "OrdinaryOnly", "Recipe::CallMethod"),
    37: (("tail_call_method", 3, 2, 0, "NPop"), "OrdinaryOnly", "Recipe::TailCallMethod"),
    38: (("array_from", 3, 0, 1, "NPop"), "OrdinaryOnly", "Recipe::ArrayFrom"),
    39: (("apply", 3, 3, 1, "U16"), "OrdinaryOnly", "Recipe::Apply"),
    41: (("return_undef", 1, 0, 0, "None"), "OrdinaryOnly", "Recipe::ReturnUndefined"),
    48: (("throw", 1, 1, 0, "None"), "OrdinaryOnly", "Recipe::Throw"),
    49: (("throw_error", 6, 0, 0, "AtomU8"), "OrdinaryOnly", "Recipe::ThrowReadOnly"),
    111: (("to_object", 1, 1, 1, "None"), "OrdinaryOnly", "Recipe::ToObject"),
    177: (("nop", 1, 0, 0, "None"), "OrdinaryOnly", "Recipe::Nop"),
    236: (("call0", 1, 1, 1, "NPopX"), "OrdinaryOnly", "Recipe::Call"),
    237: (("call1", 1, 1, 1, "NPopX"), "OrdinaryOnly", "Recipe::Call"),
    238: (("call2", 1, 1, 1, "NPopX"), "OrdinaryOnly", "Recipe::Call"),
    239: (("call3", 1, 1, 1, "NPopX"), "OrdinaryOnly", "Recipe::Call"),
}
found_stage_boundaries = {
    raw: (pinned_descriptors[raw], registry_rows[raw][2], registry_rows[raw][3])
    for raw in expected_stage_boundaries
    if raw < len(pinned_descriptors) and raw < len(registry_rows)
}
if found_stage_boundaries != expected_stage_boundaries:
    fail(
        "function-translate-stage-boundary",
        "reviewed invocation, return-undefined, explicit-throw, and plain-call boundaries must retain their pinned descriptors and policy rows; "
        f"found {found_stage_boundaries}",
    )

stage_one_ordinary_rows = (
    6, 7, 9, 10, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27,
    28, 29, 30, 31, 32, 41, 105, 138, 139, 140, 141, 142, 143, 147, 148, 149,
    152, 154, 157, 158, 159, 160, 161, 162, 164, 167, 168, 170, 171, 172, 173,
    174, 176, 191, 233, 240, 241, 242, 243,
)
former_scalar_rows = {6, 7, 9, 10, 138, 139, 140, 141, 147, 148, 149, 176, 191}
found_stage_one_rows = tuple(
    raw
    for raw in stage_one_ordinary_rows
    if registry_rows[raw][2]
    == ("Shared" if raw in former_scalar_rows else "OrdinaryOnly")
)
if found_stage_one_rows != stage_one_ordinary_rows:
    fail(
        "function-translate-stage-one-set",
        "the reviewed stage-one 57-row ordinary cohort must remain admitted with its exact audiences; "
        f"found {found_stage_one_rows}",
    )

stage_two_plain_call_rows = (34, 236, 237, 238, 239)
found_stage_two_plain_call_rows = tuple(
    raw
    for raw, _, audience, detail in registry_rows
    if audience == "OrdinaryOnly" and detail == "Recipe::Call"
)
if found_stage_two_plain_call_rows != stage_two_plain_call_rows:
    fail(
        "function-translate-stage-two-set",
        "stage two must admit exactly raw call plus call0 through call3 as ordinary plain calls; "
        f"found {found_stage_two_plain_call_rows}",
    )

stage_three_a_invocation_rows = (
    (33, "NPop", "Recipe::Construct"),
    (36, "NPop", "Recipe::CallMethod"),
    (38, "NPop", "Recipe::ArrayFrom"),
)
stage_three_a_recipe_names = {detail for _, _, detail in stage_three_a_invocation_rows}
found_stage_three_a_invocation_rows = tuple(
    (raw, operand_format, detail)
    for raw, operand_format, audience, detail in registry_rows
    if audience == "OrdinaryOnly" and detail in stage_three_a_recipe_names
)
if found_stage_three_a_invocation_rows != stage_three_a_invocation_rows:
    fail(
        "function-translate-stage-three-a-set",
        "stage three A must admit exactly Construct raw 33, CallMethod raw 36, and ArrayFrom raw 38 with NPop operands; "
        f"found {found_stage_three_a_invocation_rows}",
    )

stage_three_b_apply_rows = ((39, "U16", "Recipe::Apply"),)
found_stage_three_b_apply_rows = tuple(
    (raw, registry_rows[raw][1], registry_rows[raw][3])
    for raw, _, _ in stage_three_b_apply_rows
    if registry_rows[raw][2] == "OrdinaryOnly"
)
if found_stage_three_b_apply_rows != stage_three_b_apply_rows:
    fail(
        "function-translate-stage-three-b-set",
        "stage three B must admit exactly raw 39 apply with its U16 operand and typed Apply recipe; "
        f"found {found_stage_three_b_apply_rows}",
    )

stage_three_c_tail_rows = (
    (35, "NPop", "Recipe::TailCall"),
    (37, "NPop", "Recipe::TailCallMethod"),
)
found_stage_three_c_tail_rows = tuple(
    (raw, registry_rows[raw][1], registry_rows[raw][3])
    for raw, _, _ in stage_three_c_tail_rows
    if registry_rows[raw][2] == "OrdinaryOnly"
)
if found_stage_three_c_tail_rows != stage_three_c_tail_rows:
    fail(
        "function-translate-stage-three-c-set",
        "stage three C must admit exactly raw 35 TailCall and raw 37 TailCallMethod as distinct OrdinaryOnly NPop recipes; "
        f"found {found_stage_three_c_tail_rows}",
    )

stage_three_d_throw_rows = ((48, "None", "Recipe::Throw"),)
found_stage_three_d_throw_rows = tuple(
    (raw, registry_rows[raw][1], registry_rows[raw][3])
    for raw, _, _ in stage_three_d_throw_rows
    if registry_rows[raw][2] == "OrdinaryOnly"
)
if found_stage_three_d_throw_rows != stage_three_d_throw_rows:
    fail(
        "function-translate-stage-three-d-set",
        "stage three D must admit exactly operand-free raw 48 Throw as an OrdinaryOnly recipe; "
        f"found {found_stage_three_d_throw_rows}",
    )

stage_three_e_throw_error_rows = ((49, "AtomU8", "Recipe::ThrowReadOnly"),)
found_stage_three_e_throw_error_rows = tuple(
    (raw, registry_rows[raw][1], registry_rows[raw][3])
    for raw, _, _ in stage_three_e_throw_error_rows
    if registry_rows[raw][2] == "OrdinaryOnly"
)
if found_stage_three_e_throw_error_rows != stage_three_e_throw_error_rows:
    fail(
        "function-translate-stage-three-e-set",
        "stage three E must admit exactly raw 49 throw_error through its AtomU8 subtype-checked ThrowReadOnly recipe; "
        f"found {found_stage_three_e_throw_error_rows}",
    )

stage_three_f_nop_rows = ((177, "None", "Recipe::Nop"),)
found_stage_three_f_nop_rows = tuple(
    (raw, registry_rows[raw][1], registry_rows[raw][3])
    for raw, _, _ in stage_three_f_nop_rows
    if registry_rows[raw][2] == "OrdinaryOnly"
)
if found_stage_three_f_nop_rows != stage_three_f_nop_rows:
    fail(
        "function-translate-stage-three-f-set",
        "stage three F must admit exactly operand-free raw 177 nop as an OrdinaryOnly Nop recipe; "
        f"found {found_stage_three_f_nop_rows}",
    )

stage_three_g_object_rows = ((11, "None", "Recipe::Object"),)
found_stage_three_g_object_rows = tuple(
    (raw, registry_rows[raw][1], registry_rows[raw][3])
    for raw, _, _ in stage_three_g_object_rows
    if registry_rows[raw][2] == "OrdinaryOnly"
)
if found_stage_three_g_object_rows != stage_three_g_object_rows:
    fail(
        "function-translate-stage-three-g-set",
        "stage three G must admit exactly operand-free raw 11 object as an OrdinaryOnly Object recipe; "
        f"found {found_stage_three_g_object_rows}",
    )

stage_three_h_to_object_rows = ((111, "None", "Recipe::ToObject"),)
found_stage_three_h_to_object_rows = tuple(
    (raw, registry_rows[raw][1], registry_rows[raw][3])
    for raw, _, _ in stage_three_h_to_object_rows
    if registry_rows[raw][2] == "OrdinaryOnly"
)
if found_stage_three_h_to_object_rows != stage_three_h_to_object_rows:
    fail(
        "function-translate-stage-three-h-set",
        "stage three H must admit exactly operand-free raw 111 to_object as an OrdinaryOnly ToObject recipe; "
        f"found {found_stage_three_h_to_object_rows}",
    )

stage_three_i_push_this_rows = ((8, "None", "Recipe::PushThis"),)
found_stage_three_i_push_this_rows = tuple(
    (raw, registry_rows[raw][1], registry_rows[raw][3])
    for raw, _, _ in stage_three_i_push_this_rows
    if registry_rows[raw][2] == "OrdinaryOnly"
)
if found_stage_three_i_push_this_rows != stage_three_i_push_this_rows:
    fail(
        "function-translate-stage-three-i-set",
        "stage three I must admit exactly operand-free raw 8 push_this as an OrdinaryOnly PushThis recipe; "
        f"found {found_stage_three_i_push_this_rows}",
    )

stage_three_i_deferred_rows = {
    47: (("return_async", 1, 1, 0, "None"), "Blocked", "Completion"),
    112: (("to_propkey", 1, 1, 1, "None"), "Blocked", "ValueConstruction"),
}
found_stage_three_i_deferred_rows = {
    raw: (pinned_descriptors[raw], registry_rows[raw][2], registry_rows[raw][3])
    for raw in stage_three_i_deferred_rows
    if raw < len(pinned_descriptors) and raw < len(registry_rows)
}
if found_stage_three_i_deferred_rows != stage_three_i_deferred_rows:
    fail(
        "function-translate-stage-three-i-set",
        "raw 47 return_async and raw 112 to_propkey must remain outside the Stage3I admission; "
        f"found {found_stage_three_i_deferred_rows}",
    )

blocker_count_tokens = """
    InvalidSentinel 1 ValueConstruction 5 FunctionGraph 2 Completion 1
    EvalOrModule 3 Binding 7 Property 16 ObjectConstruction 15
    LexicalEnvironment 25 ControlFlow 4 DynamicScope 9 Iteration 11 Suspension 5
    Operator 4 Specialized 3
""".split()
expected_blocker_counts = dict(
    zip(blocker_count_tokens[::2], map(int, blocker_count_tokens[1::2]))
)
found_blocker_counts = {
    blocker: sum(
        audience == "Blocked" and detail == blocker
        for _, _, audience, detail in registry_rows
    )
    for blocker in expected_blocker_counts
}
unexpected_blockers = sorted(
    {
        detail
        for _, _, audience, detail in registry_rows
        if audience == "Blocked" and detail not in expected_blocker_counts
    }
)
if found_blocker_counts != expected_blocker_counts or unexpected_blockers:
    fail(
        "function-translate-registry-blockers",
        "the blocked frontier must retain all 15 nonempty typed categories and their exact counts, with the retired Invocation and Exception buckets absent; "
        f"found {found_blocker_counts} with unexpected {unexpected_blockers}",
    )
blocked_registry_mapping = "\n".join(
    f"{raw}:{detail}"
    for raw, _, audience, detail in registry_rows
    if audience == "Blocked"
)
blocked_registry_mapping_hash = hashlib.sha256(
    blocked_registry_mapping.encode("utf-8")
).hexdigest()
if blocked_registry_mapping_hash != (
    "2de312c2cdde1ff16d3ccf8bce0a3e1feb7857ed6ab4b603f1d991f1d315b9dd"
):
    fail(
        "function-translate-registry-blockers",
        "every blocked raw opcode must retain its reviewed typed blocker, not only the aggregate category counts; "
        f"mapping sha256 {blocked_registry_mapping_hash}",
    )

dto_blocker_code, _, _ = unique_braced_item(
    dto_production_code,
    re.compile(
        r"\bpub[ \t\n]*\([ \t\n]*in[ \t\n]+crate[ \t\n]*::[ \t\n]*runtime"
        r"[ \t\n]*::[ \t\n]*binary_object[ \t\n]*\)[ \t\n]+enum"
        r"[ \t\n]+TranslationBlocker[ \t\n]*\{"
    ),
    "function-translate-dto-shape",
    "TranslationBlocker enum",
)
if enum_variant_names(dto_blocker_code) != list(expected_blocker_counts):
    fail(
        "function-translate-dto-shape",
        "TranslationBlocker must retain the 15 reviewed ordered nonempty categories and no retired Invocation or Exception bucket; "
        f"found {enum_variant_names(dto_blocker_code)}",
    )

dto_forbidden = re.compile(
    r"\b(?:FunctionId|ImageAtom|PinnedAtomId|PinnedOpcode|NativeAtomRef|"
    r"NativeCodePlan|NativeInstruction|NativeOperands|WireString|ImageCode|"
    r"ImageInstructionSpan|ImageRelocation|byte_pc|native_pc|operand_pc|"
    r"target_pc|source_pc|Runtime|Context|RuntimeError|Instruction|JsString|"
    r"Value|Vm|VmHost|RawValue|Heap|HeapObject|ObjectRef|UnlinkedFunction)\b|"
    r"\b(?:raw(?:_opcode)?|opcode_(?:raw|byte)|(?:atom|function|image|instruction)_id)"
    r"[ \t\n]*:"
)
for match in dto_forbidden.finditer(dto_production_code):
    fail(
        "function-translate-dto-representation",
        "sanitized translation DTOs must not retain native PCs, image identities, wire strings, or executable runtime types; found "
        + location(function_translate_dto_relative, dto_source, match.start()),
    )

instruction_new_item, _, _ = unique_braced_item(
    dto_production_code,
    re.compile(
        r"\bpub[ \t\n]*\([ \t\n]*super[ \t\n]*\)[ \t\n]+const[ \t\n]+fn"
        r"[ \t\n]+new[ \t\n]*\([ \t\n]*audience[ \t\n]*:"
    ),
    "function-translate-dto-representation",
    "FunctionInstruction constructor",
)
expected_instruction_new = (
    "pub(super) const fn new( audience: InstructionAudience, "
    "diagnostic: OperationDiagnostic, operation: FunctionOp<'image>, ) -> Self { "
    "Self { audience, diagnostic, operation, } }"
)
if " ".join(instruction_new_item.split()) != expected_instruction_new:
    fail(
        "function-translate-dto-representation",
        "FunctionInstruction::new must store its sanitized audience, diagnostic, and operation unchanged",
    )

forbidden_dto_traits = {
    "AtomStringSpelling": {"Eq", "Hash", "PartialEq"},
    "AtomOperandValue": {"Eq", "Hash", "PartialEq"},
    "AtomOperand": {"Eq", "Hash", "PartialEq"},
    "FunctionOp": {"Eq", "Hash", "PartialEq"},
    "FunctionInstruction": {"Debug", "Eq", "Hash", "PartialEq"},
    "FunctionCode": {"Debug", "Default", "Eq", "Hash", "PartialEq"},
    "OperationDiagnostic": {"Debug", "Hash"},
}
dto_attribute_pattern = re.compile(
    r"(?P<attributes>(?:#[ \t\n]*\[[^]]*\][ \t\n]*)+)"
    r"(?:pub(?:[ \t\n]*\([^)]*\))?[ \t\n]+)?(?:enum|struct)"
    r"[ \t\n]+(?P<name>" + "|".join(forbidden_dto_traits) + r")\b"
)
for item in dto_attribute_pattern.finditer(dto_production_code):
    name = item.group("name")
    leaked = forbidden_dto_traits[name] & set(
        re.findall(r"\b[A-Za-z_][A-Za-z0-9_]*\b", item.group("attributes"))
    )
    if leaked:
        fail(
            "function-translate-dto-representation",
            f"{name} must not derive representation-revealing traits {sorted(leaked)}",
        )
for name, traits in forbidden_dto_traits.items():
    for trait in traits:
        if re.search(
            rf"\bimpl\b[^{{;]*\b(?:fmt[ \t\n]*::[ \t\n]*)?{trait}\b[^{{;]*\bfor[ \t\n]+{name}\b",
            dto_production_code,
        ):
            fail(
                "function-translate-dto-representation",
                f"{name} must not implement representation-revealing trait {trait}",
            )

translate_special_case_pattern = re.compile(
    r"(?i:\btest262\b|\bfixture(?:_[A-Za-z0-9_]+)?\b|"
    r"\b(?:source|input|bytes)_[A-Za-z0-9_]*(?:hash|digest|sha_?(?:1|256|512))\b)|"
    r"\b(?:input|bytes)[ \t\n]*(?:\.[A-Za-z_][A-Za-z0-9_]*[ \t\n]*\([^;\n]*\))*"
    r"\.[ \t\n]*(?:contains|starts_with|ends_with|windows)[ \t\n]*\("
)
for relative, source in (
    (function_translate_relative, function_translate_production_source),
    (function_translate_capability_relative, capability_production_source),
    (function_translate_dto_relative, dto_production_source),
):
    special_case = translate_special_case_pattern.search(source)
    if special_case is not None:
        fail(
            "function-translate-special-casing",
            "function translation must not admit by Test262 path, fixture identity, source bytes, or digest; found "
            + location(relative, source, special_case.start()),
        )

translate_native_item, translate_native_start, translate_native_end = unique_braced_item(
    function_translate_production_code,
    re.compile(r"\bfn[ \t\n]+translate_native_plan\b[^{};]*\{"),
    "function-translate-control-flow",
    "native-plan translation function",
)
translate_lower_item, _, _ = unique_braced_item(
    function_translate_production_code,
    re.compile(r"\bfn[ \t\n]+lower_operation\b[^{};]*\{"),
    "function-translate-semantic-dispatch",
    "recipe-based semantic lowering function",
)
translate_error_kind_code, _, _ = unique_braced_item(
    function_translate_production_code,
    re.compile(r"\benum[ \t\n]+FunctionTranslateErrorKind[ \t\n]*\{"),
    "function-translate-apply-admission",
    "translation error kind",
)
if enum_variant_names(translate_error_kind_code) != [
    "NativePlan",
    "RegistryDrift",
    "AllocationFailed",
    "InstructionCountOverflow",
    "InvalidBranchTarget",
    "AtomProjectionInvariant",
    "NonCanonicalApplyMagic",
    "UnadmittedThrowErrorSubtype",
]:
    fail(
        "function-translate-apply-admission",
        "translation errors must retain dedicated noncanonical-Apply and throw_error-subtype operand classes",
    )
translate_target_item, _, _ = unique_braced_item(
    function_translate_production_code,
    re.compile(r"\bfn[ \t\n]+operation_for_target\b[^{};]*\{"),
    "function-translate-atom-order",
    "target-filtered operand materializer",
)
pending_expansion_item, _, _ = unique_braced_item(
    function_translate_production_code,
    re.compile(r"\bstruct[ \t\n]+PendingExpansion\b[^{};]*\{"),
    "function-translate-expansion",
    "fixed-capacity pending expansion carrier",
)
if " ".join(pending_expansion_item.split()) != (
    "struct PendingExpansion<'image> { "
    "operations: [Option<PendingOperation<'image>>; 4], len: u8, }"
):
    fail(
        "function-translate-expansion",
        "pending expansions must remain a fixed four-slot carrier with no per-op allocation",
    )
pending_expansion_impl, _, _ = unique_braced_item(
    function_translate_production_code,
    re.compile(
        r"\bimpl[ \t\n]*<[ \t\n]*'image[ \t\n]*>[ \t\n]+PendingExpansion"
        r"[ \t\n]*<[ \t\n]*'image[ \t\n]*>[ \t\n]*\{"
    ),
    "function-translate-expansion",
    "pending expansion implementation",
)
normalized_pending_expansion = " ".join(pending_expansion_impl.split())
found_pending_initializers = re.findall(
    r"operations: \[([^]]+)\], len: ([1-4]),",
    normalized_pending_expansion,
)
expected_pending_initializers = [
    ("Some(operation), None, None, None", "1"),
    ("Some(first), Some(second), None, None", "2"),
    ("Some(first), Some(second), Some(third), None", "3"),
    ("Some(first), Some(second), Some(third), Some(fourth)", "4"),
]
pending_helper_fragments = (
    "const fn len(&self) -> usize { self.len as usize }",
    "fn into_operations(self) -> impl Iterator<Item = PendingOperation<'image>> { "
    "self.operations .into_iter() .take(usize::from(self.len)) .flatten() }",
)
if (
    found_pending_initializers != expected_pending_initializers
    or any(
        normalized_pending_expansion.count(fragment) != 1
        for fragment in pending_helper_fragments
    )
):
    fail(
        "function-translate-expansion",
        "pending expansion constructors, length, and iterator must preserve exact source order",
    )
normalized_translate_target = " ".join(translate_target_item.split())
if (
    normalized_translate_target.count(
        "if target.accepts(audience) { lower_operation(recipe, operands) } else { "
        "Ok(PendingExpansion::one(PendingOperation::Ready( FunctionOp::OutsideTarget, ))) }"
    )
    != 1
):
    fail(
        "function-translate-atom-order",
        "target audience rejection must precede all operand materialization",
    )
normalized_translate_lower = " ".join(translate_lower_item.split())
require_normalized_code_sha256(
    "function-translate-semantic-dispatch",
    "lower_operation must remain one alias-free typed Recipe/operand match with its unique ready publisher",
    translate_lower_item,
    "25b42c25b4e7b6a14304a4b37456420725fa45dd7f6f145be02b096e2718d2c2",
)
if normalized_translate_lower.count(
    "let ready = |operation| Ok(PendingExpansion::one(PendingOperation::Ready(operation)));"
) != 1:
    fail(
        "function-translate-semantic-dispatch",
        "the single-step lowering closure must pass its typed operation through unchanged",
    )
lower_match = re.search(
    r"match \(recipe, operands\) \{ (.*) _ => Err\(",
    normalized_translate_lower,
)
lowering_arm_matches = [] if lower_match is None else list(re.finditer(
    r"\( *(Recipe::.*?), (NativeOperands::.*?)(?:,)? *\) => (.*?)(?= \( *Recipe::|$)",
    lower_match.group(1).rstrip(", "),
))
found_single_step_arms = []
for arm in lowering_arm_matches:
    recipe_text, operands_text, body = arm.groups()
    if "StackRecipe::" in recipe_text and "StackRecipe::Direct" not in recipe_text:
        continue
    found_single_step_arms.append((recipe_text, operands_text, body.rstrip(", ")))
single_step_rows = """
Recipe::Nop @ NativeOperands::None @ ready(FunctionOp::Nop)
Recipe::Object @ NativeOperands::None @ ready(FunctionOp::Object)
Recipe::ToObject @ NativeOperands::None @ ready(FunctionOp::ToObject)
Recipe::PushThis @ NativeOperands::None @ ready(FunctionOp::PushThis)
Recipe::PushI32 @ NativeOperands::I32(value) | NativeOperands::NoneInt(value) @ { ready(FunctionOp::PushI32(*value)) }
Recipe::PushI32 @ NativeOperands::I8(value) @ { ready(FunctionOp::PushI32(i32::from(*value))) }
Recipe::PushI32 @ NativeOperands::I16(value) @ { ready(FunctionOp::PushI32(i32::from(*value))) }
Recipe::PushConstant @ NativeOperands::Const(index) @ { ready(FunctionOp::PushConstant(*index)) }
Recipe::PushConstant @ NativeOperands::Const8(index) @ { ready(FunctionOp::PushConstant(u32::from(*index))) }
Recipe::PushAtom @ NativeOperands::Atom(atom) @ { ready(FunctionOp::PushAtom(project_atom(*atom)?)) }
Recipe::PushUndefined @ NativeOperands::None @ ready(FunctionOp::PushUndefined)
Recipe::PushNull @ NativeOperands::None @ ready(FunctionOp::PushNull)
Recipe::PushFalse @ NativeOperands::None @ ready(FunctionOp::PushBool(false))
Recipe::PushTrue @ NativeOperands::None @ ready(FunctionOp::PushBool(true))
Recipe::PushBigIntI32 @ NativeOperands::I32(value) @ { ready(FunctionOp::PushBigIntI32(*value)) }
Recipe::PushEmptyString @ NativeOperands::None @ ready(FunctionOp::PushEmptyString)
Recipe::Stack(capability::StackRecipe::Direct(operation)) @ NativeOperands::None @ { ready(FunctionOp::Stack(operation)) }
Recipe::Unary(operation) @ NativeOperands::None @ ready(FunctionOp::Unary(operation))
Recipe::PostDec @ NativeOperands::None @ ready(FunctionOp::PostDec)
Recipe::PostInc @ NativeOperands::None @ ready(FunctionOp::PostInc)
Recipe::GetLocal @ NativeOperands::Loc(index) | NativeOperands::NoneLoc(index) @ { ready(FunctionOp::GetLocal(*index)) }
Recipe::GetLocal @ NativeOperands::Loc8(index) @ { ready(FunctionOp::GetLocal(u16::from(*index))) }
Recipe::PutLocal @ NativeOperands::Loc(index) | NativeOperands::NoneLoc(index) @ { ready(FunctionOp::PutLocal(*index)) }
Recipe::PutLocal @ NativeOperands::Loc8(index) @ { ready(FunctionOp::PutLocal(u16::from(*index))) }
Recipe::SetLocal @ NativeOperands::Loc(index) | NativeOperands::NoneLoc(index) @ { ready(FunctionOp::SetLocal(*index)) }
Recipe::SetLocal @ NativeOperands::Loc8(index) @ { ready(FunctionOp::SetLocal(u16::from(*index))) }
Recipe::GetArgument @ NativeOperands::Arg(index) | NativeOperands::NoneArg(index) @ { ready(FunctionOp::GetArgument(*index)) }
Recipe::PutArgument @ NativeOperands::Arg(index) | NativeOperands::NoneArg(index) @ { ready(FunctionOp::PutArgument(*index)) }
Recipe::SetArgument @ NativeOperands::Arg(index) | NativeOperands::NoneArg(index) @ { ready(FunctionOp::SetArgument(*index)) }
Recipe::Binary(operation) @ NativeOperands::None @ ready(FunctionOp::Binary(operation))
Recipe::Predicate(operation) @ NativeOperands::None @ { ready(FunctionOp::Predicate(operation)) }
Recipe::IfFalse @ NativeOperands::Label(label) @ Ok(PendingExpansion::one( PendingOperation::IfFalse(label.target_instruction()), ))
Recipe::IfFalse @ NativeOperands::Label8(label) @ Ok(PendingExpansion::one( PendingOperation::IfFalse(label.target_instruction()), ))
Recipe::IfTrue @ NativeOperands::Label(label) @ Ok(PendingExpansion::one( PendingOperation::IfTrue(label.target_instruction()), ))
Recipe::IfTrue @ NativeOperands::Label8(label) @ Ok(PendingExpansion::one( PendingOperation::IfTrue(label.target_instruction()), ))
Recipe::Goto @ NativeOperands::Label(label) @ Ok(PendingExpansion::one( PendingOperation::Goto(label.target_instruction()), ))
Recipe::Goto @ NativeOperands::Label8(label) @ Ok(PendingExpansion::one( PendingOperation::Goto(label.target_instruction()), ))
Recipe::Goto @ NativeOperands::Label16(label) @ Ok(PendingExpansion::one( PendingOperation::Goto(label.target_instruction()), ))
Recipe::Call @ NativeOperands::NPop(argument_count) | NativeOperands::NPopX(argument_count) @ ready(FunctionOp::Call(*argument_count))
Recipe::TailCall @ NativeOperands::NPop(argument_count) @ { ready(FunctionOp::TailCall(*argument_count)) }
Recipe::Construct @ NativeOperands::NPop(argument_count) @ { ready(FunctionOp::Construct(*argument_count)) }
Recipe::CallMethod @ NativeOperands::NPop(argument_count) @ { ready(FunctionOp::CallMethod(*argument_count)) }
Recipe::TailCallMethod @ NativeOperands::NPop(argument_count) @ { ready(FunctionOp::TailCallMethod(*argument_count)) }
Recipe::ArrayFrom @ NativeOperands::NPop(argument_count) @ { ready(FunctionOp::ArrayFrom(*argument_count)) }
Recipe::Apply @ NativeOperands::U16(0) @ { ready(FunctionOp::Apply(FunctionApplyKind::Call)) }
Recipe::Apply @ NativeOperands::U16(1) @ { ready(FunctionOp::Apply(FunctionApplyKind::Construct)) }
Recipe::Apply @ NativeOperands::U16(magic) @ { Err(FunctionTranslateError::non_canonical_apply_magic(*magic)) }
Recipe::Return @ NativeOperands::None @ ready(FunctionOp::Return)
Recipe::ReturnUndefined @ NativeOperands::None @ ready(FunctionOp::ReturnUndefined)
Recipe::Throw @ NativeOperands::None @ ready(FunctionOp::Throw)
Recipe::ThrowReadOnly @ NativeOperands::AtomU8 { atom, value: 0 } @ { ready(FunctionOp::ThrowReadOnly(project_atom(*atom)?)) }
Recipe::ThrowReadOnly @ NativeOperands::AtomU8 { value, .. } @ Err( FunctionTranslateError::unadmitted_throw_error_subtype(*value), )
""".strip().splitlines()
expected_single_step_arms = [
    tuple(row.split(" @ ", 2)) for row in single_step_rows
]
if len(lowering_arm_matches) != 58 or found_single_step_arms != expected_single_step_arms:
    fail(
        "function-translate-semantic-dispatch",
        "lower_operation must retain all 58 reviewed Recipe/operand arms, including operand-free Nop, Object, ToObject, and PushThis, terminal tail invocations, explicit throw, and subtype-0 ThrowReadOnly, with each exact normalized RHS payload expression; "
        f"found {found_single_step_arms}",
    )
apply_magic_error_contracts = (
    (
        "non_canonical_apply_magic",
        "fn non_canonical_apply_magic(magic: u16) -> Self { Self { kind: FunctionTranslateErrorKind::NonCanonicalApplyMagic(magic), } }",
    ),
    (
        "unadmitted_throw_error_subtype",
        "fn unadmitted_throw_error_subtype(subtype: u8) -> Self { Self { kind: FunctionTranslateErrorKind::UnadmittedThrowErrorSubtype(subtype), } }",
    ),
    (
        "is_unadmitted_operand_error",
        "fn is_unadmitted_operand_error(&self) -> bool { matches!( self.kind, FunctionTranslateErrorKind::NonCanonicalApplyMagic(_) | FunctionTranslateErrorKind::UnadmittedThrowErrorSubtype(_) ) }",
    ),
)
for function_name, expected in apply_magic_error_contracts:
    item, _, _ = unique_braced_item(
        function_translate_code,
        re.compile(rf"\bfn[ \t\n]+{function_name}\b[^{{}};]*\{{"),
        "function-translate-apply-admission",
        function_name,
    )
    if " ".join(item.split()) != expected:
        fail(
            "function-translate-apply-admission",
            f"{function_name} must retain the exact noncanonical-Apply and throw_error-subtype error classification",
        )
stack_expansion_pattern = re.compile(
    r"\(Recipe::Stack\(capability::StackRecipe::(\w+)\), NativeOperands::None\)"
    r" => \{ Ok\(PendingExpansion::(two|three|four)\((.*?)\)\) \}"
)
stack_expansion_matches = stack_expansion_pattern.findall(normalized_translate_lower)
found_stack_expansions = {
    recipe: (arity, tuple(re.findall(r"FunctionStackOp::(\w+)", body)))
    for recipe, arity, body in stack_expansion_matches
}
expected_stack_expansions = {
    "Nip1": ("two", ("Perm3", "Nip")),
    "Dup2": ("three", ("Dup1", "Dup", "Perm3")),
    "Swap2": ("two", ("Rot4Left", "Rot4Left")),
    "Rot3Left": ("two", ("Perm3", "Swap")),
    "Rot3Right": ("two", ("Swap", "Perm3")),
    "Rot5Left": ("four", ("Perm4", "Perm4", "Perm5", "Rot4Left")),
}
if (
    len(stack_expansion_matches) != 6
    or found_stack_expansions != expected_stack_expansions
    or normalized_translate_lower.count(
        "(Recipe::ReturnUndefined, NativeOperands::None) => ready(FunctionOp::ReturnUndefined),"
    ) != 1
):
    fail(
        "function-translate-expansion",
        "the six stack expansions and single zero-stack ReturnUndefined lowering must retain their exact allocation-free shape; "
        f"found {found_stack_expansions}",
    )
if re.search(
    r"\b(?:OperationDiagnostic|opcode|diagnostic|mnemonic)\b|\.[ \t\n]*name[ \t\n]*\(",
    translate_lower_item,
):
    fail(
        "function-translate-semantic-dispatch",
        "semantic lowering must dispatch only on typed recipes and operands, never opcode names or diagnostics",
    )
opcode_name_uses = list(
    re.finditer(r"\bopcode[ \t\n]*\.[ \t\n]*name[ \t\n]*\(", function_translate_production_code)
)
if len(opcode_name_uses) != 2 or any(
    not (translate_native_start <= match.start() < translate_native_end)
    for match in opcode_name_uses
):
    fail(
        "function-translate-diagnostic-boundary",
        "opcode.name() may appear only in the two compatibility-diagnostic constructions inside translate_native_plan",
    )
normalized_translate_native = " ".join(translate_native_item.split())
diagnostic_fragments = (
    "FunctionTranslateError::registry_drift( opcode.name(), expected_shape, descriptor_shape, decoded_shape, )",
    "let diagnostic = OperationDiagnostic::new(opcode.name(), expected_shape);",
)
if any(normalized_translate_native.count(fragment) != 1 for fragment in diagnostic_fragments):
    fail(
        "function-translate-diagnostic-boundary",
        "the opcode mnemonic may feed only registry-drift and rejection-text diagnostics",
    )

branch_fragments = (
    "source_to_output.push(output_index);",
    "PendingOperation::Ready(operation) => operation,",
    "PendingOperation::IfFalse(target) => { FunctionOp::IfFalse(resolve_target(&source_to_output, target)?) }",
    "PendingOperation::IfTrue(target) => { FunctionOp::IfTrue(resolve_target(&source_to_output, target)?) }",
    "PendingOperation::Goto(target) => { FunctionOp::Goto(resolve_target(&source_to_output, target)?) }",
    "output.push(FunctionInstruction::new( instruction.audience, instruction.diagnostic, operation, ));",
)
if any(normalized_translate_native.count(fragment) != 1 for fragment in branch_fragments):
    fail(
        "function-translate-control-flow",
        "translation must retain one source-to-output map and resolve all three branch kinds through it in the second pass",
    )
source_map_fragments = (
    "let mut output_len = 0_usize;",
    "let output_index = u32::try_from(output_len)",
    "source_to_output.push(output_index);",
    "let (audience, expansion) = match row.policy",
    "output_len = output_len .checked_add(expansion.len())",
    "pending.push(PendingInstruction",
)
source_map_offsets = [
    normalized_translate_native.find(fragment) for fragment in source_map_fragments
]
output_len_writes = re.findall(
    r"\boutput_len[ \t\n]*(?:[+*/%&|^-]?=)(?!=)",
    translate_native_item,
)
output_index_writes = re.findall(
    r"\boutput_index[ \t\n]*(?:[+*/%&|^-]?=)(?!=)",
    translate_native_item,
)
if (
    any(offset < 0 for offset in source_map_offsets)
    or source_map_offsets != sorted(source_map_offsets)
    or any(normalized_translate_native.count(fragment) != 1 for fragment in source_map_fragments)
    or len(output_len_writes) != 2
    or len(output_index_writes) != 1
    or normalized_translate_native.count(
        "pending .try_reserve_exact(plan.instructions().len())"
    ) != 1
):
    fail(
        "function-translate-control-flow",
        "each physical source must reserve one pending slot and map to the cumulative output length before its expansion",
    )
resolve_target_item, _, _ = unique_braced_item(
    function_translate_production_code,
    re.compile(r"\bfn[ \t\n]+resolve_target\b[^{};]*\{"),
    "function-translate-control-flow",
    "sanitized branch-target resolver",
)
normalized_resolve_target = " ".join(resolve_target_item.split())
expected_resolve_target = "fn resolve_target( source_to_output: &[u32], target_instruction: u32, ) -> Result<u32, FunctionTranslateError> { source_to_output .get(target_instruction as usize) .copied() .ok_or_else(FunctionTranslateError::invalid_branch_target) }"
if normalized_resolve_target != expected_resolve_target:
    fail(
        "function-translate-control-flow",
        "branch targets must remain bounds-checked instruction indexes in the sanitized output map",
    )

translate_atom_item, _, _ = unique_braced_item(
    function_translate_production_code,
    re.compile(r"\bfn[ \t\n]+project_atom\b[^{};]*\{"),
    "function-translate-atom-order",
    "borrowed semantic atom projection",
)
require_normalized_code_sha256(
    "function-translate-atom-order",
    "project_atom must preserve String spelling and input-table provenance without allocation or class aliasing",
    translate_atom_item,
    "b61b799820c1c462fc2000f1ecd8b9e0698446da2e9b70f0bbb9f13de099b58c",
)
if re.search(
    r"\b(?:Vec|Box|String)[ \t\n]*(?:::|<)|\b(?:try_reserve|reserve|collect|"
    r"to_owned|to_vec|into_boxed_slice)[ \t\n]*\(",
    translate_atom_item,
):
    fail(
        "function-translate-atom-order",
        "atom translation must remain allocation-free so scalar rejection and OOM ordering stay unchanged",
    )
translate_atom_classes = re.findall(
    r"NativeAtomClass[ \t\n]*::[ \t\n]*([A-Za-z_][A-Za-z0-9_]*)",
    translate_atom_item,
)
if translate_atom_classes != ["Null", "Index", "String", "Private", "Symbol"]:
    fail(
        "function-translate-atom-order",
        "borrowed atom projection must preserve all five semantic identity classes in order; "
        f"found {translate_atom_classes}",
    )

ordinary_leaf_relative = "src/runtime/binary_object/ordinary_leaf.rs"
ordinary_leaf_source = read_source(ordinary_leaf_relative)
ordinary_leaf_code = rust_code_only(ordinary_leaf_source)
ordinary_leaf_production_code = ordinary_leaf_code.split("#[cfg(test)]", 1)[0]
ordinary_leaf_production_source = ordinary_leaf_source.split("#[cfg(test)]", 1)[0]
ordinary_visibility = (
    r"pub[ \t\n]*\([ \t\n]*in[ \t\n]+crate[ \t\n]*::[ \t\n]*runtime"
    r"[ \t\n]*\)"
)

ordinary_visible_item_pattern = re.compile(
    r"\b(?P<visibility>pub(?:[ \t\n]*\([^)]*\))?)[ \t\n]+"
    r"(?:(?:const|async|unsafe|extern)[ \t\n]+)*"
    r"(?P<kind>enum|struct|union|trait|type|fn|const|static|use|mod)[ \t\n]+"
    r"(?P<name>[A-Za-z_][A-Za-z0-9_]*)"
)
ordinary_visible_items = [
    (
        " ".join(match.group("visibility").split()),
        match.group("kind"),
        match.group("name"),
    )
    for match in ordinary_visible_item_pattern.finditer(ordinary_leaf_production_code)
]
expected_ordinary_visible_items = [
    ("pub(in crate::runtime)", kind, name)
    for kind, name in (
        entry.split(":", 1)
        for entry in """
        struct:RootFunctionConstantSelector fn:from_zero_based fn:zero_based
        struct:OrdinaryLeafMetadataDraft fn:argument_count fn:defined_argument_count
        fn:local_count fn:max_stack fn:is_strict fn:has_simple_parameter_list
        fn:has_prototype fn:allows_new_target fn:allows_arguments fn:strip_variable_debug
        enum:DetachedPrimitive struct:DetachedAtomName fn:into_units
        enum:OrdinaryLeafOp enum:OrdinaryLeafApplyKind enum:OrdinaryLeafStackOp
        enum:OrdinaryLeafUnaryOp enum:OrdinaryLeafBinaryOp enum:OrdinaryLeafPredicateOp
        struct:OrdinaryLeafDraft fn:metadata fn:constants fn:code fn:into_parts
        enum:OrdinaryLeafReadError
        fn:decode_trusted_ordinary_leaf
        """.split()
    )
]
if ordinary_visible_items != expected_ordinary_visible_items:
    fail(
        "ordinary-leaf-visible-item-set",
        "ordinary_leaf.rs may expose only the reviewed selector, owned semantic DTOs, accessors, error, and decoder to runtime; "
        f"found {ordinary_visible_items}",
    )

ordinary_top_level_item_pattern = re.compile(
    r"(?m)^[ \t]*(?:pub(?:[ \t\n]*\([^)]*\))?[ \t\n]+)?"
    r"(?P<kind>struct|enum|union|trait|type|mod)[ \t\n]+"
    r"(?P<name>[A-Za-z_][A-Za-z0-9_]*)"
)
ordinary_top_level_items = [
    (match.group("kind"), match.group("name"))
    for match in ordinary_top_level_item_pattern.finditer(ordinary_leaf_production_code)
    if ordinary_leaf_production_code[:match.start()].count("{")
    == ordinary_leaf_production_code[:match.start()].count("}")
]
expected_ordinary_top_level_items = [
    tuple(entry.split(":", 1))
    for entry in """
    struct:RootFunctionConstantSelector struct:OrdinaryLeafMetadataDraft
    enum:DetachedPrimitive struct:DetachedAtomName enum:OrdinaryLeafOp
    enum:OrdinaryLeafApplyKind enum:OrdinaryLeafStackOp
    enum:OrdinaryLeafUnaryOp enum:OrdinaryLeafBinaryOp enum:OrdinaryLeafPredicateOp
    struct:OrdinaryLeafDraft enum:OrdinaryLeafReadError struct:AdmissionLimits
    struct:InputAtomLedger
    """.split()
]
if ordinary_top_level_items != expected_ordinary_top_level_items:
    fail(
        "ordinary-leaf-top-level-item-set",
        "ordinary_leaf.rs must retain exactly the reviewed DTO and private state types, with no module, trait, alias, union, or helper type escape; "
        f"found {ordinary_top_level_items}",
    )

ordinary_leaf_op_code, _, _ = unique_braced_item(
    ordinary_leaf_production_code,
    re.compile(r"\benum[ \t\n]+OrdinaryLeafOp[ \t\n]*\{"),
    "ordinary-leaf-operation-shape",
    "OrdinaryLeafOp enum",
)
expected_ordinary_leaf_op_variants = """
    Nop Object ToObject PushThis PushI32 PushConst PushUndefined PushNull PushBool PushBigIntI32 PushEmptyString
    Stack Unary PostDec PostInc GetLocal PutLocal SetLocal GetArgument PutArgument
    SetArgument Binary Predicate IfFalse IfTrue Goto Call TailCall Construct
    CallMethod TailCallMethod ArrayFrom Apply Return ReturnUndefined Throw ThrowReadOnly
""".split()
ordinary_invocation_payloads = {
    name: [
        " ".join(payload.split())
        for payload in re.findall(
            rf"\b{name}[ \t\n]*\(([^()]*)\)[ \t\n]*,", ordinary_leaf_op_code
        )
    ]
    for name in invocation_variant_names
}
if (
    enum_variant_names(ordinary_leaf_op_code) != expected_ordinary_leaf_op_variants
    or any(ordinary_invocation_payloads[name] != ["u16"]
           for name in counted_invocation_variant_names)
    or ordinary_invocation_payloads["Apply"] != ["OrdinaryLeafApplyKind"]
):
    fail(
        "ordinary-leaf-operation-shape",
        "OrdinaryLeafOp must retain the exact reviewed inventory with operand-free Nop/Object/ToObject/PushThis, distinct counted u16 invocation payloads, a typed Apply kind, and operand-free Throw; "
        f"found {enum_variant_names(ordinary_leaf_op_code)} with invocation payloads {ordinary_invocation_payloads}",
    )
ordinary_throw_read_only_payloads = [
    " ".join(payload.split())
    for payload in re.findall(
        r"\bThrowReadOnly[ \t\n]*\(([^()]*)\)[ \t\n]*,",
        ordinary_leaf_op_code,
    )
]
if ordinary_throw_read_only_payloads != ["DetachedAtomName"]:
    fail(
        "ordinary-leaf-operation-shape",
        "OrdinaryLeafOp::ThrowReadOnly must retain exactly one owned DetachedAtomName payload",
    )

ordinary_apply_kind_code, _, _ = unique_braced_item(
    ordinary_leaf_production_code,
    re.compile(
        ordinary_visibility
        + r"[ \t\n]+enum[ \t\n]+OrdinaryLeafApplyKind[ \t\n]*\{"
    ),
    "ordinary-leaf-apply-kind",
    "owned ordinary apply kind",
)
if enum_variant_names(ordinary_apply_kind_code) != ["Call", "Construct"]:
    fail(
        "ordinary-leaf-apply-kind",
        "OrdinaryLeafApplyKind must expose only call and construct semantics; "
        f"found {enum_variant_names(ordinary_apply_kind_code)}",
    )

ordinary_top_level_functions = [
    match.group("name")
    for match in re.finditer(
        r"(?m)^[ \t]*(?:pub(?:[ \t\n]*\([^)]*\))?[ \t\n]+)?fn"
        r"[ \t\n]+(?P<name>[A-Za-z_][A-Za-z0-9_]*)",
        ordinary_leaf_production_code,
    )
    if ordinary_leaf_production_code[:match.start()].count("{")
    == ordinary_leaf_production_code[:match.start()].count("}")
]
expected_ordinary_top_level_functions = """
    decode_trusted_ordinary_leaf admit_image preflight_constants project_primitive
    copy_wire_string copy_bigint lower_code validate_push_this_protocol lower_operation copy_read_only_name lower_constant lower_local
    lower_argument validate_ir_target unsupported_operation classify_translation_error
    unadmitted classify_image_error classify_atom_error classify_wire_error
    classify_data_error classify_envelope_error classify_code_error
""".split()
if ordinary_top_level_functions != expected_ordinary_top_level_functions:
    fail(
        "ordinary-leaf-helper-set",
        "ordinary_leaf.rs production free-function ownership drifted from the reviewed helper set; "
        f"found {ordinary_top_level_functions}",
    )

ordinary_lower_code, _, _ = unique_braced_item(
    ordinary_leaf_production_code,
    re.compile(r"\bfn[ \t\n]+lower_code\b[^{};]*\{"),
    "ordinary-leaf-translated-code",
    "sanitized ordinary code consumer",
)
ordinary_lower_operation, _, _ = unique_braced_item(
    ordinary_leaf_production_code,
    re.compile(r"\bfn[ \t\n]+lower_operation\b[^{};]*\{"),
    "ordinary-leaf-translated-code",
    "sanitized ordinary operation lowering",
)
require_normalized_code_sha256(
    "ordinary-leaf-translated-code",
    "ordinary lower_operation must remain one alias-free exhaustive typed handoff",
    ordinary_lower_operation,
    "77d12d77999176f90897eb647ef49229847c4504475525cdc237b87ba2b33da2",
)
ordinary_handoff_rows = """
FunctionOp::Nop @ Ok(OrdinaryLeafOp::Nop)
FunctionOp::Object @ Ok(OrdinaryLeafOp::Object)
FunctionOp::ToObject @ Ok(OrdinaryLeafOp::ToObject)
FunctionOp::PushThis @ Ok(OrdinaryLeafOp::PushThis)
FunctionOp::PushI32(value) @ Ok(OrdinaryLeafOp::PushI32(*value))
FunctionOp::PushConstant(index) @ lower_constant(*index, constant_count)
FunctionOp::PushUndefined @ Ok(OrdinaryLeafOp::PushUndefined)
FunctionOp::PushNull @ Ok(OrdinaryLeafOp::PushNull)
FunctionOp::PushBool(value) @ Ok(OrdinaryLeafOp::PushBool(*value))
FunctionOp::PushBigIntI32(value) @ Ok(OrdinaryLeafOp::PushBigIntI32(*value))
FunctionOp::PushEmptyString @ Ok(OrdinaryLeafOp::PushEmptyString)
FunctionOp::Stack(operation) @ Ok(OrdinaryLeafOp::Stack(match operation { FunctionStackOp::Drop => OrdinaryLeafStackOp::Drop, FunctionStackOp::Nip => OrdinaryLeafStackOp::Nip, FunctionStackOp::Dup => OrdinaryLeafStackOp::Dup, FunctionStackOp::Dup1 => OrdinaryLeafStackOp::Dup1, FunctionStackOp::Dup3 => OrdinaryLeafStackOp::Dup3, FunctionStackOp::Insert2 => OrdinaryLeafStackOp::Insert2, FunctionStackOp::Insert3 => OrdinaryLeafStackOp::Insert3, FunctionStackOp::Insert4 => OrdinaryLeafStackOp::Insert4, FunctionStackOp::Perm3 => OrdinaryLeafStackOp::Perm3, FunctionStackOp::Perm4 => OrdinaryLeafStackOp::Perm4, FunctionStackOp::Perm5 => OrdinaryLeafStackOp::Perm5, FunctionStackOp::Swap => OrdinaryLeafStackOp::Swap, FunctionStackOp::Rot4Left => OrdinaryLeafStackOp::Rot4Left, }))
FunctionOp::Unary(operation) @ Ok(OrdinaryLeafOp::Unary(match operation { FunctionUnaryOp::Neg => OrdinaryLeafUnaryOp::Neg, FunctionUnaryOp::Plus => OrdinaryLeafUnaryOp::Plus, FunctionUnaryOp::Dec => OrdinaryLeafUnaryOp::Dec, FunctionUnaryOp::Inc => OrdinaryLeafUnaryOp::Inc, FunctionUnaryOp::BitNot => OrdinaryLeafUnaryOp::BitNot, FunctionUnaryOp::LogicalNot => OrdinaryLeafUnaryOp::LogicalNot, FunctionUnaryOp::TypeOf => OrdinaryLeafUnaryOp::TypeOf, }))
FunctionOp::PostDec @ Ok(OrdinaryLeafOp::PostDec)
FunctionOp::PostInc @ Ok(OrdinaryLeafOp::PostInc)
FunctionOp::GetLocal(index) @ lower_local(*index, local_count, OrdinaryLeafOp::GetLocal)
FunctionOp::PutLocal(index) @ lower_local(*index, local_count, OrdinaryLeafOp::PutLocal)
FunctionOp::SetLocal(index) @ lower_local(*index, local_count, OrdinaryLeafOp::SetLocal)
FunctionOp::GetArgument(index) @ { lower_argument(*index, argument_count, OrdinaryLeafOp::GetArgument) }
FunctionOp::PutArgument(index) @ { lower_argument(*index, argument_count, OrdinaryLeafOp::PutArgument) }
FunctionOp::SetArgument(index) @ { lower_argument(*index, argument_count, OrdinaryLeafOp::SetArgument) }
FunctionOp::Binary(operation) @ Ok(OrdinaryLeafOp::Binary(match operation { FunctionBinaryOp::Add => OrdinaryLeafBinaryOp::Add, FunctionBinaryOp::Sub => OrdinaryLeafBinaryOp::Sub, FunctionBinaryOp::Mul => OrdinaryLeafBinaryOp::Mul, FunctionBinaryOp::Div => OrdinaryLeafBinaryOp::Div, FunctionBinaryOp::Mod => OrdinaryLeafBinaryOp::Mod, FunctionBinaryOp::Pow => OrdinaryLeafBinaryOp::Pow, FunctionBinaryOp::Shl => OrdinaryLeafBinaryOp::Shl, FunctionBinaryOp::Sar => OrdinaryLeafBinaryOp::Sar, FunctionBinaryOp::Shr => OrdinaryLeafBinaryOp::Shr, FunctionBinaryOp::LessThan => OrdinaryLeafBinaryOp::LessThan, FunctionBinaryOp::LessThanOrEqual => OrdinaryLeafBinaryOp::LessThanOrEqual, FunctionBinaryOp::GreaterThan => OrdinaryLeafBinaryOp::GreaterThan, FunctionBinaryOp::GreaterThanOrEqual => OrdinaryLeafBinaryOp::GreaterThanOrEqual, FunctionBinaryOp::Equal => OrdinaryLeafBinaryOp::Equal, FunctionBinaryOp::NotEqual => OrdinaryLeafBinaryOp::NotEqual, FunctionBinaryOp::StrictEqual => OrdinaryLeafBinaryOp::StrictEqual, FunctionBinaryOp::StrictNotEqual => OrdinaryLeafBinaryOp::StrictNotEqual, FunctionBinaryOp::BitAnd => OrdinaryLeafBinaryOp::BitAnd, FunctionBinaryOp::BitXor => OrdinaryLeafBinaryOp::BitXor, FunctionBinaryOp::BitOr => OrdinaryLeafBinaryOp::BitOr, }))
FunctionOp::Predicate(operation) @ Ok(OrdinaryLeafOp::Predicate(match operation { FunctionPredicateOp::IsUndefinedOrNull => OrdinaryLeafPredicateOp::IsUndefinedOrNull, FunctionPredicateOp::IsUndefined => OrdinaryLeafPredicateOp::IsUndefined, FunctionPredicateOp::IsNull => OrdinaryLeafPredicateOp::IsNull, FunctionPredicateOp::TypeOfIsUndefined => OrdinaryLeafPredicateOp::TypeOfIsUndefined, FunctionPredicateOp::TypeOfIsFunction => OrdinaryLeafPredicateOp::TypeOfIsFunction, }))
FunctionOp::IfFalse(target) @ { validate_ir_target(*target, instruction_count).map(OrdinaryLeafOp::IfFalse) }
FunctionOp::IfTrue(target) @ { validate_ir_target(*target, instruction_count).map(OrdinaryLeafOp::IfTrue) }
FunctionOp::Goto(target) @ { validate_ir_target(*target, instruction_count).map(OrdinaryLeafOp::Goto) }
FunctionOp::Call(argument_count) @ Ok(OrdinaryLeafOp::Call(*argument_count))
FunctionOp::TailCall(argument_count) @ Ok(OrdinaryLeafOp::TailCall(*argument_count))
FunctionOp::Construct(argument_count) @ Ok(OrdinaryLeafOp::Construct(*argument_count))
FunctionOp::CallMethod(argument_count) @ Ok(OrdinaryLeafOp::CallMethod(*argument_count))
FunctionOp::TailCallMethod(argument_count) @ { Ok(OrdinaryLeafOp::TailCallMethod(*argument_count)) }
FunctionOp::ArrayFrom(element_count) @ Ok(OrdinaryLeafOp::ArrayFrom(*element_count))
FunctionOp::Apply(kind) @ Ok(OrdinaryLeafOp::Apply(match kind { FunctionApplyKind::Call => OrdinaryLeafApplyKind::Call, FunctionApplyKind::Construct => OrdinaryLeafApplyKind::Construct, }))
FunctionOp::Return @ Ok(OrdinaryLeafOp::Return)
FunctionOp::ReturnUndefined @ Ok(OrdinaryLeafOp::ReturnUndefined)
FunctionOp::Throw @ Ok(OrdinaryLeafOp::Throw)
FunctionOp::ThrowReadOnly(atom) @ { copy_read_only_name(atom).map(OrdinaryLeafOp::ThrowReadOnly) }
""".strip().splitlines()
expected_ordinary_handoff = [tuple(row.split(" @ ", 1)) for row in ordinary_handoff_rows]
found_ordinary_handoff = rustfmt_match_arms(ordinary_lower_operation, "FunctionOp::")
if found_ordinary_handoff != expected_ordinary_handoff:
    fail(
        "ordinary-leaf-translated-code",
        "all 37 sanitized operations must retain their exact ordinary-leaf payload and typed-family mapping; "
        f"found {found_ordinary_handoff}",
    )
if (
    len(re.findall(r"\binstruction[ \t\n]*\.[ \t\n]*supports_ordinary[ \t\n]*\(", ordinary_lower_code)) != 1
    or len(re.findall(r"\binstruction[ \t\n]*\.[ \t\n]*operation[ \t\n]*\(", ordinary_lower_code)) != 2
    or re.search(r"\b(?:NativeCodePlan|NativeOperands|PinnedOpcode|opcode)\b", ordinary_lower_code)
    or re.search(r"\b(?:OperationDiagnostic|diagnostic|mnemonic)\b", ordinary_lower_operation)
):
    fail(
        "ordinary-leaf-translated-code",
        "ordinary_leaf must filter the sanitized audience before typed lowering and must not consult native opcodes or diagnostics while lowering",
    )

ordinary_unsupported_item, unsupported_start, unsupported_end = unique_braced_item(
    ordinary_leaf_production_code,
    re.compile(r"\bfn[ \t\n]+unsupported_operation\b[^{};]*\{"),
    "function-translate-diagnostic-boundary",
    "ordinary rejection-text formatter",
)
ordinary_translate_error_classifier, _, _ = unique_braced_item(
    ordinary_leaf_production_code,
    re.compile(r"\bfn[ \t\n]+classify_translation_error\b[^{};]*\{"),
    "ordinary-leaf-apply-admission",
    "ordinary translation-error classifier",
)
normalized_translate_error_classifier = " ".join(
    ordinary_translate_error_classifier.split()
)
apply_admission_fragments = (
    "if error.is_label_target_error() { return OrdinaryLeafReadError::Unadmitted(",
    "if error.is_unadmitted_operand_error() { return OrdinaryLeafReadError::Unadmitted(error.to_string()); }",
    "let message = error.to_string();",
    "OrdinaryLeafReadError::Internal(message)",
)
apply_admission_offsets = [
    normalized_translate_error_classifier.find(fragment)
    for fragment in apply_admission_fragments
]
if (
    any(offset < 0 for offset in apply_admission_offsets)
    or apply_admission_offsets != sorted(apply_admission_offsets)
    or any(
        normalized_translate_error_classifier.count(fragment) != 1
        for fragment in apply_admission_fragments
    )
):
    fail(
        "ordinary-leaf-apply-admission",
        "noncanonical apply operands must become Unadmitted before draft publication while internal translation failures remain Internal",
    )
if ordinary_unsupported_item and (
    len(re.findall(r"\bdiagnostic[ \t\n]*\.[ \t\n]*mnemonic[ \t\n]*\(", ordinary_unsupported_item)) != 1
    or len(re.findall(r"\bdiagnostic[ \t\n]*\.[ \t\n]*operand_shape[ \t\n]*\(", ordinary_unsupported_item)) != 1
    or len(re.findall(r"\bOrdinaryLeafReadError[ \t\n]*::[ \t\n]*Unadmitted\b", ordinary_unsupported_item)) != 1
):
    fail(
        "function-translate-diagnostic-boundary",
        "the compatibility diagnostic may be consumed only to format an ordinary-leaf Unadmitted rejection",
    )
ordinary_rejection_diagnostics = list(
    re.finditer(r"\.[ \t\n]*rejection_diagnostic[ \t\n]*\(", ordinary_leaf_production_code)
)
normalized_ordinary_lower_code = " ".join(ordinary_lower_code.split())
if (
    len(ordinary_rejection_diagnostics) != 1
    or normalized_ordinary_lower_code.count(
        "return Err(unsupported_operation(instruction.rejection_diagnostic()));"
    )
    != 1
):
    fail(
        "function-translate-diagnostic-boundary",
        "ordinary code may read one compatibility diagnostic only in the !supports_ordinary rejection branch",
    )
for accessor in ("mnemonic", "operand_shape"):
    accessor_uses = list(
        re.finditer(rf"\.[ \t\n]*{accessor}[ \t\n]*\(", ordinary_leaf_production_code)
    )
    if (
        len(accessor_uses) != 1
        or not (unsupported_start <= accessor_uses[0].start() < unsupported_end)
    ):
        fail(
            "function-translate-diagnostic-boundary",
            "diagnostic mnemonic and operand shape may be read only by the ordinary rejection-text formatter",
        )

ordinary_raw_dependency = re.search(
    r"\b(?:ImageAtom|PinnedAtomId|PinnedOpcode|NativeAtomRef|NativeCodePlan|"
    r"NativeInstruction|NativeOperands|ImageCode|ImageInstructionSpan|ImageRelocation)\b|"
    r"\.[ \t\n]*(?:as_bytes|atom_relocations)[ \t\n]*\(",
    ordinary_leaf_production_code,
)
if ordinary_raw_dependency is not None:
    fail(
        "ordinary-leaf-native-plan-boundary",
        "ordinary-leaf admission must consume only sanitized function DTOs and authenticated image APIs, never native plans, raw atom identities, code bytes, or relocation sidecars; found "
        + location(
            ordinary_leaf_relative,
            ordinary_leaf_source,
            ordinary_raw_dependency.start(),
        ),
    )

ordinary_special_case_pattern = re.compile(
    r"(?i:\btest262\b|\bfixture(?:_[A-Za-z0-9_]+)?\b|"
    r"\b(?:source|input|bytes)_[A-Za-z0-9_]*(?:hash|digest|sha_?(?:1|256|512))\b)|"
    r"\b(?:input|bytes)[ \t\n]*(?:\.[A-Za-z_][A-Za-z0-9_]*[ \t\n]*\([^;\n]*\))*"
    r"\.[ \t\n]*(?:contains|starts_with|ends_with|windows)[ \t\n]*\(",
)
ordinary_special_case = ordinary_special_case_pattern.search(ordinary_leaf_production_source)
if ordinary_special_case is not None:
    fail(
        "ordinary-leaf-special-casing",
        "ordinary-leaf production admission must remain structural and must not dispatch on Test262, fixture, digest, or exact input-byte identity; found "
        + location(
            ordinary_leaf_relative,
            ordinary_leaf_source,
            ordinary_special_case.start(),
        ),
    )

scalar_script_relative = "src/runtime/binary_object/scalar_script.rs"
scalar_script_source = read_source(scalar_script_relative)
scalar_script_code = rust_code_only(scalar_script_source)
scalar_visibility = (
    r"pub[ \t\n]*\([ \t\n]*in[ \t\n]+crate[ \t\n]*::[ \t\n]*runtime"
    r"[ \t\n]*\)"
)
scalar_noncopy_derive = (
    r"#[ \t\n]*\[[ \t\n]*derive[ \t\n]*\([ \t\n]*Clone[ \t\n]*,"
    r"[ \t\n]*Debug[ \t\n]*,[ \t\n]*Eq[ \t\n]*,"
    r"[ \t\n]*PartialEq[ \t\n]*\)[ \t\n]*\]"
)
scalar_value_draft_pattern = re.compile(
    rf"{scalar_noncopy_derive}[ \t\n]*{scalar_visibility}"
    r"[ \t\n]+enum[ \t\n]+ScalarValueDraft[ \t\n]*\{"
    r"[ \t\n]*Undefined[ \t\n]*,"
    r"[ \t\n]*Null[ \t\n]*,"
    r"[ \t\n]*Bool[ \t\n]*\([ \t\n]*bool[ \t\n]*\)[ \t\n]*,"
    r"[ \t\n]*Int[ \t\n]*\([ \t\n]*i32[ \t\n]*\)[ \t\n]*,"
    r"[ \t\n]*Float64Bits[ \t\n]*\([ \t\n]*u64[ \t\n]*\)[ \t\n]*,"
    r"[ \t\n]*BigIntI32[ \t\n]*\([ \t\n]*i32[ \t\n]*\)[ \t\n]*,"
    r"[ \t\n]*BigIntBytes[ \t\n]*\([ \t\n]*Box[ \t\n]*<"
    r"[ \t\n]*\[[ \t\n]*u8[ \t\n]*\][ \t\n]*>[ \t\n]*\)[ \t\n]*,"
    r"[ \t\n]*EmptyString[ \t\n]*,"
    r"[ \t\n]*ConstantString[ \t\n]*\([ \t\n]*ScalarStringDraft[ \t\n]*\)[ \t\n]*,"
    r"[ \t\n]*AtomString[ \t\n]*\([ \t\n]*ScalarStringDraft[ \t\n]*\)[ \t\n]*,"
    r"[ \t\n]*IntegerAtomString[ \t\n]*\([ \t\n]*u32[ \t\n]*\)[ \t\n]*,?[ \t\n]*\}"
)
if len(scalar_value_draft_pattern.findall(scalar_script_code)) != 1:
    fail(
        "scalar-script-draft-shape",
        "ScalarValueDraft must retain exactly the reviewed primitive, BigInt, and provenance-typed String variants, with no precomputed unary result",
    )

scalar_unary_pattern = re.compile(
    r"#[ \t\n]*\[[ \t\n]*derive[ \t\n]*\([ \t\n]*Clone[ \t\n]*,"
    r"[ \t\n]*Copy[ \t\n]*,[ \t\n]*Debug[ \t\n]*,[ \t\n]*Eq"
    r"[ \t\n]*,[ \t\n]*PartialEq[ \t\n]*\)[ \t\n]*\]"
    rf"[ \t\n]*{scalar_visibility}[ \t\n]+enum[ \t\n]+ScalarUnaryOp"
    r"[ \t\n]*\{[ \t\n]*Neg[ \t\n]*,[ \t\n]*Plus[ \t\n]*,"
    r"[ \t\n]*Dec[ \t\n]*,[ \t\n]*Inc[ \t\n]*,[ \t\n]*BitNot"
    r"[ \t\n]*,[ \t\n]*LogicalNot[ \t\n]*,[ \t\n]*TypeOf"
    r"[ \t\n]*,?[ \t\n]*\}"
)
if len(scalar_unary_pattern.findall(scalar_script_code)) != 1:
    fail(
        "scalar-unary-operation-shape",
        "ScalarUnaryOp must retain exactly the seven reviewed ordered, Copy operation tags",
    )

scalar_unary_impl_code, scalar_unary_impl_start, scalar_unary_impl_end = unique_braced_item(
    scalar_script_code,
    re.compile(r"\bimpl[ \t\n]+ScalarUnaryOp[ \t\n]*\{"),
    "scalar-unary-operation-shape",
    "private ScalarUnaryOp implementation",
)
scalar_unary_pairs = re.findall(
    r"FunctionUnaryOp[ \t\n]*::[ \t\n]*([A-Za-z_][A-Za-z0-9_]*)"
    r"[ \t\n]*=>[ \t\n]*Self[ \t\n]*::[ \t\n]*([A-Za-z_][A-Za-z0-9_]*)",
    scalar_unary_impl_code,
)
expected_scalar_unary_pairs = [
    ("Neg", "Neg"),
    ("Plus", "Plus"),
    ("Dec", "Dec"),
    ("Inc", "Inc"),
    ("BitNot", "BitNot"),
    ("LogicalNot", "LogicalNot"),
    ("TypeOf", "TypeOf"),
]
if scalar_unary_pairs != expected_scalar_unary_pairs:
    fail(
        "scalar-unary-operation-shape",
        "ScalarUnaryOp must map the seven sanitized unary operations one-for-one; "
        f"found {scalar_unary_pairs}",
    )

scalar_string_draft_pattern = re.compile(
    rf"{scalar_noncopy_derive}[ \t\n]*{scalar_visibility}[ \t\n]+struct"
    r"[ \t\n]+ScalarStringDraft[ \t\n]*\([ \t\n]*Box[ \t\n]*<"
    r"[ \t\n]*\[[ \t\n]*u16[ \t\n]*\][ \t\n]*>[ \t\n]*\)[ \t\n]*;"
)
if len(scalar_string_draft_pattern.findall(scalar_script_code)) != 1:
    fail(
        "scalar-script-draft-shape",
        "ScalarStringDraft must be one opaque runtime-visible owned UTF-16 code-unit buffer",
    )

scalar_error_pattern = re.compile(
    rf"\b{scalar_visibility}[ \t\n]+enum[ \t\n]+ScalarScriptReadError[ \t\n]*\{{"
    r"[ \t\n]*Malformed[ \t\n]*\([ \t\n]*String[ \t\n]*\)[ \t\n]*,"
    r"[ \t\n]*Type[ \t\n]*\([ \t\n]*String[ \t\n]*\)[ \t\n]*,"
    r"[ \t\n]*Range[ \t\n]*\([ \t\n]*String[ \t\n]*\)[ \t\n]*,"
    r"[ \t\n]*JsInternal[ \t\n]*\([ \t\n]*String[ \t\n]*\)[ \t\n]*,"
    r"[ \t\n]*Unadmitted[ \t\n]*\([ \t\n]*String[ \t\n]*\)[ \t\n]*,"
    r"[ \t\n]*Resource[ \t\n]*\([ \t\n]*String[ \t\n]*\)[ \t\n]*,"
    r"[ \t\n]*Internal[ \t\n]*\([ \t\n]*String[ \t\n]*\)[ \t\n]*,?[ \t\n]*\}"
)
if len(scalar_error_pattern.findall(scalar_script_code)) != 1:
    fail(
        "scalar-script-error-shape",
        "ScalarScriptReadError must retain exactly Malformed, Type, Range, JsInternal, Unadmitted, Resource, and Internal String variants",
    )

scalar_decode_item_pattern = re.compile(
    rf"\b{scalar_visibility}[ \t\n]+fn[ \t\n]+decode_trusted_scalar_script"
    r"[ \t\n]*\([ \t\n]*(?:bytes|input)[ \t\n]*:[ \t\n]*&[ \t\n]*\[u8\]"
    r"[ \t\n]*,?[ \t\n]*\)[ \t\n]*->[ \t\n]*Result[ \t\n]*<"
    r"[ \t\n]*\([ \t\n]*ScalarValueDraft[ \t\n]*,[ \t\n]*Box"
    r"[ \t\n]*<[ \t\n]*\[[ \t\n]*ScalarUnaryOp[ \t\n]*\][ \t\n]*>"
    r"[ \t\n]*\)[ \t\n]*,[ \t\n]*ScalarScriptReadError[ \t\n]*>"
    r"[ \t\n]*\{"
)
scalar_decoder_code, _, _ = unique_braced_item(
    scalar_script_code,
    scalar_decode_item_pattern,
    "scalar-script-decoder-shape",
    "runtime-visible &[u8] to scalar draft decoder",
)
if scalar_decoder_code:
    decoder_steps = (
        list(re.finditer(r"\bdecode_bytecode_image_body[ \t\n]*\(", scalar_decoder_code)),
        list(re.finditer(
            r"\bcursor[ \t\n]*\.[ \t\n]*finish[ \t\n]*\([ \t\n]*\)",
            scalar_decoder_code,
        )),
        list(re.finditer(
            r"\badmit_image[ \t\n]*\([ \t\n]*&[ \t\n]*image[ \t\n]*\)",
            scalar_decoder_code,
        )),
    )
    decoder_step_offsets = tuple(
        step[0].start() if len(step) == 1 else -1 for step in decoder_steps
    )
    if (
        any(len(step) != 1 for step in decoder_steps)
        or decoder_step_offsets != tuple(sorted(decoder_step_offsets))
        or re.search(r"\bOk[ \t\n]*\(", scalar_decoder_code)
        or re.search(
            r"\badmit_image[ \t\n]*\([ \t\n]*&[ \t\n]*image[ \t\n]*\)"
            r"[ \t\n]*\}[ \t\n]*\Z",
            scalar_decoder_code,
        ) is None
    ):
        fail(
            "scalar-script-decoder-shape",
            "decode_trusted_scalar_script must uniquely complete decode_bytecode_image_body, cursor.finish(), and final admit_image(&image), without an alternate Ok result",
        )

scalar_opcode_declarations = re.findall(
    r"(?m)^[ \t]*const[ \t]+(OP_[A-Z0-9_]+)\b",
    scalar_script_code.split("#[cfg(test)]", 1)[0],
)
if scalar_opcode_declarations:
    fail(
        "scalar-script-opcode-set",
        "scalar-script production admission must not retain raw opcode constants after translation centralization; "
        f"found {scalar_opcode_declarations}",
    )

scalar_push_pattern = re.compile(
    r"#[ \t\n]*\[[ \t\n]*derive[ \t\n]*\([ \t\n]*Clone[ \t\n]*,"
    r"[ \t\n]*Debug[ \t\n]*\)[ \t\n]*\][ \t\n]*enum[ \t\n]+ScalarPush"
    r"[ \t\n]*<[ \t\n]*'image[ \t\n]*>[ \t\n]*\{"
    r"[ \t\n]*Direct[ \t\n]*\([ \t\n]*ScalarValueDraft[ \t\n]*\)"
    r"[ \t\n]*,[ \t\n]*Constant[ \t\n]*\([ \t\n]*u32[ \t\n]*\)"
    r"[ \t\n]*,[ \t\n]*AtomValue[ \t\n]*\([ \t\n]*AtomOperand"
    r"[ \t\n]*<[ \t\n]*'image[ \t\n]*>[ \t\n]*\)[ \t\n]*,?[ \t\n]*\}"
)
if len(scalar_push_pattern.findall(scalar_script_code)) != 1:
    fail(
        "scalar-script-push-shape",
        "ScalarPush must retain only a direct draft, constant index, or sanitized atom operand",
    )

scalar_sequence_pattern = re.compile(
    r"#[ \t\n]*\[[ \t\n]*derive[ \t\n]*\([ \t\n]*Clone[ \t\n]*,"
    r"[ \t\n]*Debug[ \t\n]*\)[ \t\n]*\][ \t\n]*struct[ \t\n]+ScalarSequence"
    r"[ \t\n]*<[ \t\n]*'image[ \t\n]*>[ \t\n]*\{"
    r"[ \t\n]*push[ \t\n]*:[ \t\n]*ScalarPush"
    r"[ \t\n]*<[ \t\n]*'image[ \t\n]*>[ \t\n]*,"
    r"[ \t\n]*unary_ops[ \t\n]*:[ \t\n]*Box[ \t\n]*<"
    r"[ \t\n]*\[[ \t\n]*ScalarUnaryOp[ \t\n]*\][ \t\n]*>"
    r"[ \t\n]*,?[ \t\n]*\}"
)
if len(scalar_sequence_pattern.findall(scalar_script_code)) != 1:
    fail(
        "scalar-script-sequence-shape",
        "ScalarSequence must retain one sanitized value push and one owned ordered unary-operation slice",
    )

scalar_copy_pattern = re.compile(
    r"#[ \t\n]*\[[ \t\n]*derive[ \t\n]*\([^\]]*\bCopy\b[^\]]*\)"
    r"[ \t\n]*\](?:[ \t\n]*#[ \t\n]*\[[^\]]*\])*[ \t\n]*"
    rf"(?:{scalar_visibility}[ \t\n]+)?enum[ \t\n]+"
    r"(?:ScalarValueDraft|ScalarStringDraft|ScalarPush|ScalarSequence)\b|"
    r"\bimpl[ \t\n]+Copy[ \t\n]+for[ \t\n]+"
    r"(?:ScalarValueDraft|ScalarStringDraft|ScalarPush|ScalarSequence)\b"
)
if scalar_copy_pattern.search(scalar_script_code):
    fail(
        "scalar-script-draft-shape",
        "the scalar draft, push, and sequence discriminators must not regain Copy semantics around owned BigInt bytes",
    )

scalar_sequence_code, _, _ = unique_braced_item(
    scalar_script_code,
    re.compile(r"\bfn[ \t\n]+decode_scalar_sequence\b[^{};]*\{"),
    "scalar-script-translated-code",
    "sanitized scalar sequence decoder",
)
normalized_scalar_sequence = " ".join(scalar_sequence_code.split())
scalar_sequence_fragments = (
    ".any(|instruction| !instruction.supports_scalar())",
    "!matches!(set_completion.operation(), FunctionOp::SetLocal(0))",
    "!matches!(return_value.operation(), FunctionOp::Return)",
    "let FunctionOp::Unary(operation) = instruction.operation() else",
    "unary_ops.push(ScalarUnaryOp::from_translated(*operation));",
    ".and_then(|instruction| decode_scalar_push(instruction.into_operation()))",
)
if (
    any(normalized_scalar_sequence.count(fragment) != 1 for fragment in scalar_sequence_fragments)
    or re.search(r"\b(?:NativeCodePlan|NativeOperands|PinnedOpcode|opcode|diagnostic|mnemonic)\b", scalar_sequence_code)
    or re.search(r"\bunary_ops[ \t\n]*\.[ \t\n]*(?:dedup|insert|last|reverse|sort)\b", scalar_sequence_code)
):
    fail(
        "scalar-script-translated-code",
        "scalar sequence admission must filter sanitized audiences, preserve set-local-zero/return and unary order, and consume the owned push without native or diagnostic dispatch",
    )

scalar_native_production_code = scalar_script_code.split("#[cfg(test)]", 1)[0]
raw_scalar_decoder_dependency = re.search(
    r"\b(?:ImageAtom|PinnedAtomId|PinnedOpcode|NativeAtomRef|NativeCodePlan|"
    r"NativeInstruction|NativeOperands|ImageCode|ImageInstructionSpan|ImageRelocation)\b|"
    r"\.[ \t\n]*(?:as_bytes|atom_relocations)[ \t\n]*\(",
    scalar_native_production_code,
)
if raw_scalar_decoder_dependency is not None:
    fail(
        "scalar-script-native-plan-decoder",
        "scalar admission must consume only sanitized function DTOs, never native plans, raw atom identities, archival code bytes, or relocation sidecars; found "
        + location(
            scalar_script_relative,
            scalar_script_source,
            raw_scalar_decoder_dependency.start(),
        ),
    )

bigint_copy_pattern = re.compile(
    r"\bfn[ \t\n]+copy_bigint_bytes[ \t\n]*\([ \t\n]*bytes[ \t\n]*:"
    r"[ \t\n]*&[ \t\n]*\[[ \t\n]*u8[ \t\n]*\][ \t\n]*\)"
    r"[ \t\n]*->[ \t\n]*Result[ \t\n]*<[ \t\n]*Box[ \t\n]*<"
    r"[ \t\n]*\[[ \t\n]*u8[ \t\n]*\][ \t\n]*>[ \t\n]*,"
    r"[ \t\n]*ScalarScriptReadError[ \t\n]*>[ \t\n]*\{",
    re.DOTALL,
)
bigint_copy_code, _, _ = unique_braced_item(
    scalar_script_code,
    bigint_copy_pattern,
    "scalar-script-bigint-copy",
    "fallible canonical BigInt byte-copy helper",
)
if bigint_copy_code:
    expected_bigint_copy_source = """
        fn copy_bigint_bytes(bytes: &[u8]) -> Result<Box<[u8]>, ScalarScriptReadError> {
            let mut copy = Vec::new();
            copy.try_reserve_exact(bytes.len()).map_err(|_| {
                ScalarScriptReadError::Internal("could not allocate the scalar BigInt draft".into())
            })?;
            copy.extend_from_slice(bytes);
            Ok(copy.into_boxed_slice())
        }
    """
    if (
        " ".join(bigint_copy_code.split())
        != " ".join(rust_code_only(expected_bigint_copy_source).split())
    ):
        fail(
            "scalar-script-bigint-copy",
            "copy_bigint_bytes must perform one exact fallible reserve, byte-for-byte copy, and boxed ownership transfer",
        )

normalized_scalar_script = " ".join(scalar_script_code.split())
scalar_string_fragments = (
    "pub(in crate::runtime) fn into_units(self) -> Box<[u16]> { self.0 }",
    "WireString::Narrow(bytes) => { copy_utf16(bytes.iter().copied().map(u16::from), bytes.len()) }",
    "WireString::Wide(units) => copy_utf16(units.iter().copied(), units.len()),",
)
utf16_copy_shape = re.compile(
    r"\bcopy[ \t\n]*\.[ \t\n]*try_reserve_exact[ \t\n]*\([ \t\n]*length"
    r"[ \t\n]*\).*?ScalarScriptReadError[ \t\n]*::[ \t\n]*JsInternal"
    r".*?\bcopy[ \t\n]*\.[ \t\n]*extend[ \t\n]*\([ \t\n]*units"
    r"[ \t\n]*\)[ \t\n]*;[ \t\n]*Ok[ \t\n]*\([ \t\n]*ScalarStringDraft"
    r"[ \t\n]*\([ \t\n]*copy[ \t\n]*\.[ \t\n]*into_boxed_slice",
    re.DOTALL,
)
if (
    any(normalized_scalar_script.count(fragment) != 1 for fragment in scalar_string_fragments)
    or len(utf16_copy_shape.findall(scalar_script_code)) != 1
):
    fail(
        "scalar-script-string-copy",
        "String drafts must cross the reader boundary once as exact, fallibly copied UTF-16 code units",
    )
if re.search(r"\b(?:try_)?from_utf8\b|\bdecode_utf8\b", scalar_script_code):
    fail(
        "scalar-script-string-copy",
        "BC5 narrow Strings are Latin-1 code units and must not pass through a UTF-8 decoder",
    )

scalar_visible_item_pattern = re.compile(
    r"\b(?P<visibility>pub(?:[ \t\n]*\([^)]*\))?)[ \t\n]+"
    r"(?P<kind>enum|struct|union|trait|type|fn|const|static|use|mod)[ \t\n]+"
    r"(?P<name>[A-Za-z_][A-Za-z0-9_]*)"
)
scalar_visible_items = [
    (
        " ".join(match.group("visibility").split()),
        match.group("kind"),
        match.group("name"),
    )
    for match in scalar_visible_item_pattern.finditer(scalar_script_code)
]
expected_scalar_visible_items = [
    ("pub(in crate::runtime)", "enum", "ScalarScriptReadError"),
    ("pub(in crate::runtime)", "enum", "ScalarUnaryOp"),
    ("pub(in crate::runtime)", "enum", "ScalarValueDraft"),
    ("pub(in crate::runtime)", "struct", "ScalarStringDraft"),
    ("pub(in crate::runtime)", "fn", "into_units"),
    ("pub(in crate::runtime)", "fn", "decode_trusted_scalar_script"),
]
if sorted(scalar_visible_items) != sorted(expected_scalar_visible_items):
    fail(
        "scalar-script-visible-item-set",
        "scalar_script.rs may expose only the reviewed value and unary DTOs, String draft, error, and decoder to runtime; "
        f"found {scalar_visible_items}",
    )

scalar_production_code = scalar_script_code.split("#[cfg(test)]", 1)[0]
scalar_top_level_item_pattern = re.compile(
    r"(?m)^[ \t]*(?:pub(?:[ \t\n]*\([^)]*\))?[ \t\n]+)?"
    r"(?P<kind>struct|enum|union|trait|type|mod)[ \t\n]+"
    r"(?P<name>[A-Za-z_][A-Za-z0-9_]*)"
)
scalar_top_level_items = [
    (match.group("kind"), match.group("name"))
    for match in scalar_top_level_item_pattern.finditer(scalar_production_code)
    if scalar_production_code[:match.start()].count("{")
    == scalar_production_code[:match.start()].count("}")
]
expected_scalar_top_level_items = [
    ("enum", "ScalarValueDraft"),
    ("enum", "ScalarUnaryOp"),
    ("struct", "ScalarStringDraft"),
    ("enum", "ScalarPush"),
    ("struct", "ScalarSequence"),
    ("enum", "ScalarScriptReadError"),
    ("struct", "AdmissionLimits"),
]
if scalar_top_level_items != expected_scalar_top_level_items:
    fail(
        "scalar-script-top-level-item-set",
        "scalar_script.rs must retain exactly the reviewed production DTO and private state types, with no module, trait, alias, union, or helper type escape; "
        f"found {scalar_top_level_items}",
    )

scalar_top_level_function_pattern = re.compile(
    r"(?m)^[ \t]*(?:pub(?:[ \t\n]*\([^)]*\))?[ \t\n]+)?fn"
    r"[ \t\n]+(?P<name>[A-Za-z_][A-Za-z0-9_]*)"
)
scalar_top_level_functions = [
    match.group("name")
    for match in scalar_top_level_function_pattern.finditer(scalar_production_code)
    if scalar_production_code[:match.start()].count("{")
    == scalar_production_code[:match.start()].count("}")
]
expected_scalar_top_level_functions = [
    "decode_trusted_scalar_script",
    "admit_image",
    "project_atom_string",
    "project_atom_string_spelling",
    "classify_translation_error",
    "copy_wire_string",
    "copy_utf16",
    "copy_bigint_bytes",
    "decode_scalar_sequence",
    "decode_scalar_push",
    "decode_direct_scalar_push",
    "unadmitted",
    "classify_image_error",
    "classify_atom_error",
    "classify_wire_error",
    "classify_data_error",
    "classify_envelope_error",
    "classify_code_error",
]
if scalar_top_level_functions != expected_scalar_top_level_functions:
    fail(
        "scalar-script-helper-set",
        "scalar_script.rs production free-function ownership drifted from the reviewed helper set; "
        f"found {scalar_top_level_functions}",
    )

scalar_impl_header_pattern = re.compile(r"(?m)^[ \t]*impl\b(?P<header>[^{};]*)\{")
scalar_impl_headers = [
    " ".join(match.group("header").split())
    for match in scalar_impl_header_pattern.finditer(scalar_production_code)
    if scalar_production_code[:match.start()].count("{")
    == scalar_production_code[:match.start()].count("}")
]
expected_scalar_impl_headers = [
    "ScalarUnaryOp",
    "ScalarStringDraft",
    "fmt::Display for ScalarScriptReadError",
    "std::error::Error for ScalarScriptReadError",
    "AdmissionLimits",
]
if scalar_impl_headers != expected_scalar_impl_headers:
    fail(
        "scalar-script-implementation-set",
        "scalar_script.rs must retain exactly the reviewed inherent and error-trait implementations; "
        f"found {scalar_impl_headers}",
    )

scalar_macro_invocations = re.findall(
    r"(?<![A-Za-z0-9_])((?:r#)?[A-Za-z_][A-Za-z0-9_]*)"
    r"[ \t\n]*![ \t\n]*[([{]",
    scalar_production_code,
)
if scalar_macro_invocations != [
    "write",
    "write",
    "write",
    "write",
    "write",
    "write",
    "write",
    "format",
    "matches",
    "format",
    "matches",
    "matches",
    "matches",
    "format",
    "format",
]:
    fail(
        "scalar-script-macro-set",
        "scalar_script.rs may invoke only the reviewed diagnostic and atom-predicate macros; "
        f"found {scalar_macro_invocations}",
    )

scalar_display_pattern = re.compile(
    r"\bimpl\b[^{};]*\b(?:fmt[ \t\n]*::[ \t\n]*)?Display[ \t\n]+for"
    r"[ \t\n]+ScalarScriptReadError[ \t\n]*\{",
    re.DOTALL,
)
if len(scalar_display_pattern.findall(scalar_script_code)) != 1:
    fail(
        "scalar-script-error-display",
        "ScalarScriptReadError must have exactly one direct Display implementation",
    )

if len(re.findall(r"\bReaderMode[ \t\n]*::[ \t\n]*QuickJsCompatible\b", scalar_script_code)) != 1:
    fail(
        "scalar-script-reader-mode",
        "scalar_script.rs must select QuickJsCompatible exactly once",
    )
for match in re.finditer(r"\bReaderMode[ \t\n]*::[ \t\n]*Strict\b", scalar_script_code):
    fail(
        "scalar-script-reader-mode",
        "scalar-script admission must not use Strict or Strict-plus-fallback; found "
        + location(scalar_script_relative, scalar_script_source, match.start()),
    )

scalar_admission_code, _, _ = unique_braced_item(
    scalar_production_code,
    re.compile(r"\bfn[ \t\n]+admit_image\b[^{};]*\{"),
    "scalar-script-translated-admission",
    "sanitized scalar admission function",
)
normalized_scalar_admission = " ".join(scalar_admission_code.split())
scalar_admission_fragments = (
    "let translated = translate_function(image, root, TranslationTarget::Scalar) .map_err(classify_translation_error)?;",
    "let Some(sequence) = decode_scalar_sequence(translated)? else",
    "if !matches!(&sequence.push, ScalarPush::AtomValue(_)) && image.input_atom_slot_count() != 0",
    "let ScalarSequence { push, unary_ops } = sequence;",
    "let value = match (push, function.constants())",
    "(ScalarPush::AtomValue(atom), []) => project_atom_string(image, atom)?",
    "}; Ok((value, unary_ops))",
)
scalar_admission_offsets = [
    normalized_scalar_admission.find(fragment) for fragment in scalar_admission_fragments
]
if (
    any(offset < 0 for offset in scalar_admission_offsets)
    or scalar_admission_offsets != sorted(scalar_admission_offsets)
    or any(normalized_scalar_admission.count(fragment) != 1 for fragment in scalar_admission_fragments)
    or re.search(r"\breturn[ \t\n]+Ok[ \t\n]*\(", scalar_admission_code)
):
    fail(
        "scalar-script-translated-admission",
        "scalar admission must translate once, validate the scalar shape and atom-table boundary, pair constants, then project an admitted atom before returning",
    )

normalized_scalar_production = " ".join(scalar_production_code.split())
expected_scalar_translate_import = " ".join(
    rust_code_only(
        """
        use super::function_translate::{
            AtomOperand, AtomOperandClass, FunctionCode, FunctionOp, FunctionTranslateError,
            FunctionUnaryOp, TranslationTarget, translate_function,
        };
        """
    ).split()
)
if normalized_scalar_production.count(expected_scalar_translate_import) != 1:
    fail(
        "scalar-script-translated-import",
        "scalar_script must import exactly the reviewed sanitized translation facade",
    )
direct_native_plan_import = re.search(
    r"\b(?:bytecode_image[ \t\n]*::[ \t\n]*native_plan|NativeAtomClass|"
    r"NativeAtomRef|NativeCodePlan|NativeInstruction|NativeOperands|PinnedOpcode)\b",
    scalar_production_code,
)
if direct_native_plan_import is not None:
    fail(
        "native-plan-consumer-set",
        "scalar_script must consume only sanitized translation semantics, never the native plan or pinned opcode catalog; found "
        + location(
            scalar_script_relative,
            scalar_script_source,
            direct_native_plan_import.start(),
        ),
    )
if re.findall(
    r"\bWireValue[ \t\n]*::[ \t\n]*([A-Za-z_][A-Za-z0-9_]*)",
    scalar_production_code,
) != ["Float64Bits", "BigInt", "String"]:
    fail(
        "scalar-script-constant-pairing",
        "the scalar-script path may name only the reviewed Float64, BigInt, and String pool variants",
    )
if re.search(
    r"\b(?:ImageAtom|PinnedAtomId|NativePlanError|OperationDiagnostic)\b|"
    r"\.[ \t\n]*(?:rejection_diagnostic|mnemonic|operand_shape)[ \t\n]*\(",
    scalar_production_code,
):
    fail(
        "scalar-script-translated-admission",
        "the scalar consumer may use only sanitized semantic operands and translation errors, never raw atom identities, private native-plan errors, or compatibility diagnostics",
    )

scalar_atom_classes = re.findall(
    r"\bAtomOperandClass[ \t\n]*::[ \t\n]*([A-Za-z_][A-Za-z0-9_]*)",
    scalar_production_code,
)
scalar_atom_accessor_counts = {
    accessor: len(
        re.findall(
            rf"\batom[ \t\n]*\.[ \t\n]*{accessor}[ \t\n]*\(",
            scalar_production_code,
        )
    )
    for accessor in (
        "originates_from_input_atom_table",
        "class",
        "index_value",
        "string_utf16_len",
        "string_utf16_units",
    )
}

scalar_atom_projection_code, _, _ = unique_braced_item(
    scalar_production_code,
    re.compile(r"\bfn[ \t\n]+project_atom_string\b[^{};]*\{"),
    "scalar-native-atom-consumer",
    "sanitized atom class and provenance consumer",
)
normalized_atom_projection = " ".join(scalar_atom_projection_code.split())
atom_projection_fragments = (
    "0 if atom.originates_from_input_atom_table() =>",
    "1 if !atom.originates_from_input_atom_table() =>",
    "AtomOperandClass::Null => unadmitted(",
    "AtomOperandClass::Private => unadmitted(",
    "AtomOperandClass::Symbol => unadmitted(",
    ".index_value() .map(ScalarValueDraft::IntegerAtomString)",
    "AtomOperandClass::String => project_atom_string_spelling(atom)",
)
if any(
    normalized_atom_projection.count(fragment) != 1
    for fragment in atom_projection_fragments
):
    fail(
        "scalar-native-atom-consumer",
        "scalar atom admission must retain input-slot provenance and reject Null/Private/Symbol before Index/String projection",
    )
if (
    scalar_atom_classes != ["Null", "Private", "Symbol", "Index", "String"]
    or scalar_atom_accessor_counts
    != {
        "originates_from_input_atom_table": 2,
        "class": 1,
        "index_value": 1,
        "string_utf16_len": 1,
        "string_utf16_units": 1,
    }
):
    fail(
        "scalar-native-atom-consumer",
        "scalar admission must preserve the exact sanitized input-slot provenance and Null/Private/Symbol/Index/String identity-class boundary; "
        f"found classes {scalar_atom_classes} and accessors {scalar_atom_accessor_counts}",
    )

scalar_atom_spelling_code, _, _ = unique_braced_item(
    scalar_production_code,
    re.compile(r"\bfn[ \t\n]+project_atom_string_spelling\b[^{};]*\{"),
    "scalar-native-atom-consumer",
    "sanitized atom String spelling consumer",
)
normalized_atom_spelling = " ".join(scalar_atom_spelling_code.split())
atom_spelling_fragments = (
    "let Some(length) = atom.string_utf16_len() else",
    "let Some(units) = atom.string_utf16_units() else",
    "copy_utf16(units, length).map(ScalarValueDraft::AtomString)",
)
atom_spelling_offsets = [
    normalized_atom_spelling.find(fragment) for fragment in atom_spelling_fragments
]
if (
    any(offset < 0 for offset in atom_spelling_offsets)
    or atom_spelling_offsets != sorted(atom_spelling_offsets)
    or any(normalized_atom_spelling.count(fragment) != 1 for fragment in atom_spelling_fragments)
):
    fail(
        "scalar-native-atom-consumer",
        "atom String admission must inspect the sealed UTF-16 length and iterator before the single fallible scalar copy",
    )

scalar_translation_error_code, _, _ = unique_braced_item(
    scalar_production_code,
    re.compile(r"\bfn[ \t\n]+classify_translation_error\b[^{};]*\{"),
    "scalar-script-translated-admission",
    "translation error classifier",
)
normalized_translation_error = " ".join(scalar_translation_error_code.split())
if (
    normalized_translation_error.count("if error.is_label_target_error()") != 1
    or normalized_translation_error.count("ScalarScriptReadError::Unadmitted(") != 1
):
    fail(
        "scalar-script-translated-admission",
        "invalid translated labels must remain an ordinary scalar-cohort rejection",
    )

consumer_relative = "src/runtime/binary_object_publish.rs"
consumer_path = root / consumer_relative
consumer_exists = consumer_path.exists() or consumer_path.is_symlink()
consumer_module_declarations = re.findall(
    r"(?m)^[ \t]*mod[ \t]+binary_object_publish[ \t]*;[ \t]*$",
    runtime_code,
)
consumer_public_module_declarations = re.findall(
    r"(?m)^[ \t]*pub(?:[ \t\n]*\([^)]*\))?[ \t\n]+mod"
    r"[ \t\n]+binary_object_publish[ \t\n]*;",
    runtime_code,
)
if consumer_public_module_declarations:
    fail(
        "binary-object-consumer-module",
        "binary_object_publish must remain a private runtime module",
    )
if consumer_exists:
    if consumer_path.is_symlink() or not consumer_path.is_file():
        fail("linked-source", f"{consumer_relative} must be a regular file")
        consumer_source = ""
    else:
        consumer_source = consumer_path.read_text(encoding="utf-8")
    if len(consumer_module_declarations) != 1:
        fail(
            "binary-object-consumer-module",
            "an existing binary_object_publish.rs requires exactly one private runtime module declaration",
        )
else:
    consumer_source = ""
    if consumer_module_declarations:
        fail(
            "binary-object-consumer-module",
            "runtime must not declare binary_object_publish before its reviewed source exists",
        )

consumer_code = rust_code_only(consumer_source)
consumer_facade_import_pattern = re.compile(
    r"(?m)^[ \t]*use[ \t\n]+super[ \t\n]*::[ \t\n]*binary_object"
    r"[ \t\n]*::[ \t\n]*\{(?P<body>[^{}]*)\}[ \t\n]*;"
)
consumer_facade_imports = list(consumer_facade_import_pattern.finditer(consumer_code))
if consumer_exists:
    consumer_production_code = consumer_code.split("#[cfg(test)]", 1)[0]
    consumer_top_level_item_pattern = re.compile(
        r"(?m)^[ \t]*(?:pub(?:[ \t\n]*\([^)]*\))?[ \t\n]+)?"
        r"(?P<kind>struct|enum|union|trait|type|mod)[ \t\n]+"
        r"(?P<name>[A-Za-z_][A-Za-z0-9_]*)"
    )
    consumer_top_level_items = [
        (match.group("kind"), match.group("name"))
        for match in consumer_top_level_item_pattern.finditer(consumer_production_code)
        if consumer_production_code[:match.start()].count("{")
        == consumer_production_code[:match.start()].count("}")
    ]
    if consumer_top_level_items != [("enum", "LoweredScalar")]:
        fail(
            "binary-object-consumer-top-level-item-set",
            f"{consumer_relative} may own only the reviewed LoweredScalar type and no module, trait, alias, union, or helper type escape; "
            f"found {consumer_top_level_items}",
        )

    consumer_top_level_functions = [
        match.group("name")
        for match in re.finditer(
            r"(?m)^[ \t]*(?:pub(?:[ \t\n]*\([^)]*\))?[ \t\n]+)?"
            r"(?:const[ \t\n]+)?fn"
            r"[ \t\n]+(?P<name>[A-Za-z_][A-Za-z0-9_]*)",
            consumer_production_code,
        )
        if consumer_production_code[:match.start()].count("{")
        == consumer_production_code[:match.start()].count("}")
    ]
    expected_consumer_top_level_functions = [
        "lower_scalar_value",
        "lower_scalar_string",
        "lower_bigint_constant",
        "decode_bigint_constant",
        "lower_primitive_constant",
        "lower_detached_primitive",
        "lower_ordinary_leaf_op",
        "map_ordinary_leaf_verification_error",
        "map_ordinary_leaf_read_error",
        "map_read_error",
    ]
    if consumer_top_level_functions != expected_consumer_top_level_functions:
        fail(
            "binary-object-consumer-helper-set",
            f"{consumer_relative} production free-function ownership drifted from the reviewed helper set; "
            f"found {consumer_top_level_functions}",
        )

    consumer_impl_headers = [
        " ".join(match.group("header").split())
        for match in re.finditer(
            r"(?m)^[ \t]*impl\b(?P<header>[^{};]*)\{",
            consumer_production_code,
        )
        if consumer_production_code[:match.start()].count("{")
        == consumer_production_code[:match.start()].count("}")
    ]
    if consumer_impl_headers != ["Runtime"]:
        fail(
            "binary-object-consumer-implementation-set",
            f"{consumer_relative} must retain exactly one inherent Runtime implementation and no trait or alternate owner; "
            f"found {consumer_impl_headers}",
        )

    consumer_macro_invocations = re.findall(
        r"(?<![A-Za-z0-9_])((?:r#)?[A-Za-z_][A-Za-z0-9_]*)"
        r"[ \t\n]*![ \t\n]*[([{]",
        consumer_production_code,
    )
    if consumer_macro_invocations != [
        "matches",
        "vec",
        "format",
        "format",
        "format",
        "format",
        "format",
        "format",
        "format",
    ]:
        fail(
            "binary-object-consumer-macro-set",
            f"{consumer_relative} may invoke only its reviewed constant-vector and diagnostic macros; "
            f"found {consumer_macro_invocations}",
        )
    for match in re.finditer(
        r"(?<![A-Za-z0-9_])(?:r#)?include[ \t\n]*!",
        consumer_production_code,
    ):
        fail(
            "binary-object-consumer-source-include",
            f"{consumer_relative} must not splice unscanned Rust source; found "
            + location(consumer_relative, consumer_source, match.start()),
        )

    if len(consumer_facade_imports) != 1:
        fail(
            "binary-object-consumer-import",
            f"{consumer_relative} must contain exactly one reviewed scalar/ordinary facade import",
        )
    else:
        consumer_import_items = [
            item.strip()
            for item in consumer_facade_imports[0].group("body").split(",")
            if item.strip()
        ]
        expected_consumer_facade_names = expected_scalar_facade_names | {
            "DetachedPrimitive",
            "OrdinaryLeafApplyKind",
            "OrdinaryLeafBinaryOp",
            "OrdinaryLeafOp",
            "OrdinaryLeafPredicateOp",
            "OrdinaryLeafReadError",
            "OrdinaryLeafStackOp",
            "OrdinaryLeafUnaryOp",
            "RootFunctionConstantSelector",
            "decode_trusted_ordinary_leaf",
        }
        if (
            len(consumer_import_items) != len(expected_consumer_facade_names)
            or set(consumer_import_items)
            != expected_consumer_facade_names
        ):
            fail(
                "binary-object-consumer-import",
                f"{consumer_relative} may import only the reviewed scalar/ordinary facades; "
                f"found {consumer_import_items}",
            )

    consumer_binary_mentions = list(re.finditer(r"\bbinary_object\b", consumer_code))
    if len(consumer_binary_mentions) != 1:
        fail(
            "binary-object-consumer-import",
            f"{consumer_relative} may name binary_object only in its one reviewed facade import",
        )

    safe_publication_calls = re.findall(
        r"\b(?:self|runtime)[ \t\n]*\.[ \t\n]*publish_unlinked_function[ \t\n]*\(",
        consumer_code,
    )
    if len(safe_publication_calls) != 1:
        fail(
            "binary-object-consumer-publication",
            f"{consumer_relative} must enter publish_unlinked_function exactly once",
        )
    verified_publication_calls = re.findall(
        r"\b(?:self|runtime)[ \t\n]*\.[ \t\n]*publish_verified_unlinked_function"
        r"[ \t\n]*\(",
        consumer_code,
    )
    if len(verified_publication_calls) != 1:
        fail(
            "binary-object-consumer-publication",
            f"{consumer_relative} must enter publish_verified_unlinked_function exactly once after the dedicated ordinary-leaf verifier",
        )

    lowered_scalar_pattern = re.compile(
        r"(?m)^[ \t]*enum[ \t]+LoweredScalar[ \t\n]*\{"
        r"[ \t\n]*Direct[ \t\n]*\([ \t\n]*Instruction[ \t\n]*\)[ \t\n]*,"
        r"[ \t\n]*Constant[ \t\n]*\([ \t\n]*UnlinkedConstant[ \t\n]*\)[ \t\n]*,"
        r"[ \t\n]*AtomString[ \t\n]*\([ \t\n]*UnlinkedConstant[ \t\n]*\)[ \t\n]*,"
        r"[ \t\n]*IntegerAtomString[ \t\n]*\([ \t\n]*u32[ \t\n]*\)"
        r"[ \t\n]*,?[ \t\n]*\}"
    )
    if len(lowered_scalar_pattern.findall(consumer_code)) != 1:
        fail(
            "binary-object-consumer-scalar-mapping",
            "LoweredScalar must preserve only direct, primitive-cpool, atom-cpool, and fresh integer-atom value provenance",
        )

    publication_bridge_pattern = re.compile(
        r"\bpub[ \t\n]*\([ \t\n]*super[ \t\n]*\)[ \t\n]+fn"
        r"[ \t\n]+read_trusted_scalar_script_in_realm[ \t\n]*\([^{};]*\)"
        r"[ \t\n]*->[^{;]+\{",
        re.DOTALL,
    )
    publication_bridge_code, _, _ = unique_braced_item(
        consumer_code,
        publication_bridge_pattern,
        "binary-object-consumer-publication",
        "trusted scalar publication bridge",
    )
    expected_publication_bridge_source = """
        pub(super) fn read_trusted_scalar_script_in_realm(
            &self,
            realm: ContextId,
            bytes: &[u8],
        ) -> Result<FunctionBytecodeRef, RuntimeError> {
            let (value, unary_ops) = decode_trusted_scalar_script(bytes).map_err(map_read_error)?;
            let (push, constants) = match lower_scalar_value(value)? {
                LoweredScalar::Direct(push) => (push, Vec::new()),
                LoweredScalar::Constant(constant) | LoweredScalar::AtomString(constant) => {
                    (Instruction::PushConst(0), vec![constant])
                }
                LoweredScalar::IntegerAtomString(value) => {
                    (Instruction::PushAtomValueIndex(value), Vec::new())
                }
            };
            let instruction_capacity = unary_ops.len().checked_add(3).ok_or_else(|| {
                RuntimeError::Engine(Error::internal(
                    "trusted scalar instruction count overflowed",
                ))
            })?;
            let mut instructions = Vec::new();
            instructions
                .try_reserve_exact(instruction_capacity)
                .map_err(|_| {
                    RuntimeError::Engine(Error::internal(
                        "could not allocate trusted scalar instruction draft",
                    ))
                })?;
            instructions.push(push);
            for operation in unary_ops {
                instructions.push(match operation {
                    ScalarUnaryOp::Neg => Instruction::Neg,
                    ScalarUnaryOp::Plus => Instruction::Plus,
                    ScalarUnaryOp::Dec => Instruction::Dec,
                    ScalarUnaryOp::Inc => Instruction::Inc,
                    ScalarUnaryOp::BitNot => Instruction::BitNot,
                    ScalarUnaryOp::LogicalNot => Instruction::Not,
                    ScalarUnaryOp::TypeOf => Instruction::TypeOf,
                });
            }
            instructions.push(Instruction::SetLocal(0));
            instructions.push(Instruction::Return);
            let function = UnlinkedFunction::new(
                instructions,
                constants,
                FunctionMetadata {
                    local_count: 1,
                    max_stack: 1,
                    strip_variable_debug: true,
                    ..FunctionMetadata::default()
                },
            );

            self.publish_unlinked_function(realm, function)
        }
    """
    if (
        " ".join(publication_bridge_code.split())
        != " ".join(rust_code_only(expected_publication_bridge_source).split())
    ):
        fail(
            "binary-object-consumer-publication",
            f"{consumer_relative} must publish one checked push, every authenticated unary operation in order, completion, and return before entering the ordinary verifier/publication boundary",
        )

    ordinary_publication_bridge_code, _, _ = unique_braced_item(
        consumer_production_code,
        re.compile(
            r"\bpub[ \t\n]*\([ \t\n]*super[ \t\n]*\)[ \t\n]+fn"
            r"[ \t\n]+read_trusted_ordinary_function_in_realm\b[^{};]*\{"
        ),
        "ordinary-leaf-consumer-publication",
        "trusted ordinary-leaf publication bridge",
    )
    ordinary_publication_steps = (
        re.compile(r"\bdecode_trusted_ordinary_leaf[ \t\n]*\("),
        re.compile(r"\bdraft[ \t\n]*\.[ \t\n]*into_parts[ \t\n]*\("),
        re.compile(r"\bUnlinkedFunction[ \t\n]*::[ \t\n]*new[ \t\n]*\("),
        re.compile(
            r"\bbytecode_publish[ \t\n]*::[ \t\n]*verify_unlinked_ordinary_leaf"
            r"[ \t\n]*\("
        ),
        re.compile(
            r"\bself[ \t\n]*\.[ \t\n]*publish_verified_unlinked_function"
            r"[ \t\n]*\("
        ),
        re.compile(r"\bself[ \t\n]*\.[ \t\n]*new_bytecode_closure[ \t\n]*\("),
    )
    ordinary_publication_matches = [
        list(pattern.finditer(ordinary_publication_bridge_code))
        for pattern in ordinary_publication_steps
    ]
    ordinary_publication_offsets = [
        matches[0].start() if len(matches) == 1 else -1
        for matches in ordinary_publication_matches
    ]
    if (
        any(len(matches) != 1 for matches in ordinary_publication_matches)
        or ordinary_publication_offsets != sorted(ordinary_publication_offsets)
        or any(
            ordinary_publication_bridge_code[:offset].count("{")
            - ordinary_publication_bridge_code[:offset].count("}") != 1
            for offset in ordinary_publication_offsets
        )
    ):
        fail(
            "ordinary-leaf-consumer-publication",
            "the ordinary-leaf bridge must decode and detach before constructing the draft, then run the dedicated verifier before verified publication and closure allocation",
        )
    normalized_ordinary_publication = " ".join(ordinary_publication_bridge_code.split())
    synthetic_publication_fragments = (
        "let original_constant_count = detached_constants.len();",
        "let synthetic_constant_count = detached_code .iter() .filter(",
        "let total_constant_count = original_constant_count .checked_add(synthetic_constant_count)",
        "u32::try_from(total_constant_count)",
        "for constant in detached_constants { constants.push(lower_detached_primitive(constant)?); }",
        "constants .try_reserve_exact(synthetic_constant_count)",
        "for operation in &detached_code {",
        "OrdinaryLeafOp::PushBigIntI32(value) => constants.push(lower_primitive_constant( Value::BigInt(JsBigInt::from(*value)), )?)",
        "OrdinaryLeafOp::PushEmptyString => { constants.push(UnlinkedConstant::atom_string(JsString::from_static(",
        "let mut next_synthetic_index = u32::try_from(original_constant_count)",
        "for operation in detached_code { instructions.push(lower_ordinary_leaf_op(",
        "if next_synthetic_index as usize != total_constant_count",
    )
    synthetic_publication_offsets = [
        normalized_ordinary_publication.find(fragment)
        for fragment in synthetic_publication_fragments
    ]
    if (
        any(offset < 0 for offset in synthetic_publication_offsets)
        or synthetic_publication_offsets != sorted(synthetic_publication_offsets)
        or any(
            normalized_ordinary_publication.count(fragment) != 1
            for fragment in synthetic_publication_fragments
        )
    ):
        fail(
            "ordinary-leaf-consumer-publication",
            "original constants must publish before code-ordered synthetic BigInts/empty atoms and the matching stable PushConst index pass",
        )

    consumer_runtime_impl_code, _, _ = unique_braced_item(
        consumer_production_code,
        re.compile(r"(?m)^[ \t]*impl[ \t\n]+Runtime[ \t\n]*\{"),
        "binary-object-consumer-implementation-set",
        "sole Runtime publication implementation",
    )
    consumer_runtime_methods = [
        match.group("name")
        for match in re.finditer(
            r"(?m)^[ \t]*(?:pub(?:[ \t\n]*\([^)]*\))?[ \t\n]+)?fn"
            r"[ \t\n]+(?P<name>[A-Za-z_][A-Za-z0-9_]*)",
            consumer_runtime_impl_code,
        )
        if consumer_runtime_impl_code[:match.start()].count("{")
        - consumer_runtime_impl_code[:match.start()].count("}") == 1
    ]
    if consumer_runtime_methods != [
        "read_trusted_ordinary_function_in_realm",
        "read_trusted_scalar_script_in_realm",
    ]:
        fail(
            "binary-object-consumer-implementation-set",
            f"{consumer_relative} Runtime implementation must contain only the reviewed ordinary-leaf and scalar publication bridges; found {consumer_runtime_methods}",
        )

    scalar_lowering_pattern = re.compile(
        r"\bfn[ \t\n]+lower_scalar_value[ \t\n]*\([^{};]*\)"
        r"[ \t\n]*->[^{;]+\{",
        re.DOTALL,
    )
    scalar_lowering_code, _, _ = unique_braced_item(
        consumer_code,
        scalar_lowering_pattern,
        "binary-object-consumer-scalar-mapping",
        "lower_scalar_value function",
    )
    bigint_lowering_pattern = re.compile(
        r"\bfn[ \t\n]+lower_bigint_constant[ \t\n]*\("
        r"[ \t\n]*bytes[ \t\n]*:[ \t\n]*&[ \t\n]*\[[ \t\n]*u8"
        r"[ \t\n]*\][ \t\n]*,?[ \t\n]*\)[ \t\n]*->[^{;]+\{",
        re.DOTALL,
    )
    bigint_lowering_code, _, _ = unique_braced_item(
        consumer_code,
        bigint_lowering_pattern,
        "binary-object-consumer-bigint",
        "lower_bigint_constant function",
    )
    bigint_decoder_pattern = re.compile(
        r"\bfn[ \t\n]+decode_bigint_constant[ \t\n]*\("
        r"[ \t\n]*bytes[ \t\n]*:[ \t\n]*&[ \t\n]*\[[ \t\n]*u8"
        r"[ \t\n]*\][ \t\n]*,?[ \t\n]*\)[ \t\n]*->[^{;]+\{",
        re.DOTALL,
    )
    bigint_decoder_code, _, _ = unique_braced_item(
        consumer_code,
        bigint_decoder_pattern,
        "binary-object-consumer-bigint",
        "decode_bigint_constant function",
    )
    scalar_constant_pattern = re.compile(
        r"\bfn[ \t\n]+lower_primitive_constant[ \t\n]*\([^{};]*\)"
        r"[ \t\n]*->[^{;]+\{",
        re.DOTALL,
    )
    scalar_constant_code, _, _ = unique_braced_item(
        consumer_code,
        scalar_constant_pattern,
        "binary-object-consumer-scalar-mapping",
        "lower_primitive_constant function",
    )
    scalar_string_pattern = re.compile(
        r"\bfn[ \t\n]+lower_scalar_string[ \t\n]*\([^{};]*\)"
        r"[ \t\n]*->[^{;]+\{",
        re.DOTALL,
    )
    scalar_string_code, _, _ = unique_braced_item(
        consumer_code,
        scalar_string_pattern,
        "binary-object-consumer-string",
        "lower_scalar_string function",
    )
    expected_scalar_lowering_source = """
        fn lower_scalar_value(value: ScalarValueDraft) -> Result<LoweredScalar, RuntimeError> {
            match value {
                ScalarValueDraft::Undefined => Ok(LoweredScalar::Direct(Instruction::Undefined)),
                ScalarValueDraft::Null => Ok(LoweredScalar::Direct(Instruction::Null)),
                ScalarValueDraft::Bool(false) => Ok(LoweredScalar::Direct(Instruction::PushFalse)),
                ScalarValueDraft::Bool(true) => Ok(LoweredScalar::Direct(Instruction::PushTrue)),
                ScalarValueDraft::Int(value) => Ok(LoweredScalar::Direct(Instruction::PushI32(value))),
                ScalarValueDraft::Float64Bits(bits) => {
                    lower_primitive_constant(Value::Float(f64::from_bits(bits)))
                        .map(LoweredScalar::Constant)
                }
                ScalarValueDraft::BigIntI32(value) => {
                    lower_primitive_constant(Value::BigInt(JsBigInt::from(value)))
                        .map(LoweredScalar::Constant)
                }
                ScalarValueDraft::BigIntBytes(bytes) => {
                    lower_bigint_constant(&bytes).map(LoweredScalar::Constant)
                }
                ScalarValueDraft::EmptyString => Ok(LoweredScalar::AtomString(
                    UnlinkedConstant::atom_string(JsString::from_static("")),
                )),
                ScalarValueDraft::ConstantString(value) => lower_scalar_string(value)
                    .and_then(|value| lower_primitive_constant(Value::String(value)))
                    .map(LoweredScalar::Constant),
                ScalarValueDraft::AtomString(value) => Ok(LoweredScalar::AtomString(
                    UnlinkedConstant::atom_string(lower_scalar_string(value)?),
                )),
                ScalarValueDraft::IntegerAtomString(value) => Ok(LoweredScalar::IntegerAtomString(value)),
            }
        }
    """
    expected_scalar_string_source = """
        fn lower_scalar_string(value: ScalarStringDraft) -> Result<JsString, RuntimeError> {
            JsString::try_from_utf16(value.into_units()).map_err(|error| RuntimeError::Engine(error.into()))
        }
    """
    expected_bigint_lowering_source = """
        fn lower_bigint_constant(bytes: &[u8]) -> Result<UnlinkedConstant, RuntimeError> {
            lower_primitive_constant(Value::BigInt(decode_bigint_constant(bytes)?))
        }
    """
    expected_bigint_decoder_source = """
        fn decode_bigint_constant(bytes: &[u8]) -> Result<JsBigInt, RuntimeError> {
            let (value, consumed) =
                JsBigInt::decode_bc5_signed_le(bytes, bytes.len(), bytes.len(), true)
            .map_err(|error| {
                RuntimeError::Engine(Error::internal(format!(
                    "trusted binary-object draft contained invalid canonical BigInt bytes: {error:?}"
                )))
            })?;
            if consumed != bytes.len() {
                return Err(RuntimeError::Engine(Error::internal(
                    "trusted scalar BigInt draft was not consumed exactly",
                )));
            }
            Ok(value)
        }
    """
    expected_scalar_constant_source = """
        fn lower_primitive_constant(value: Value) -> Result<UnlinkedConstant, RuntimeError> {
            UnlinkedConstant::primitive(value).map_err(|error| {
                RuntimeError::Engine(Error::internal(format!(
                    "trusted binary-object draft produced an invalid primitive constant: {error}"
                )))
            })
        }
    """
    if (
        " ".join(scalar_lowering_code.split())
        != " ".join(rust_code_only(expected_scalar_lowering_source).split())
        or re.findall(
            r"\bScalarValueDraft[ \t\n]*::[ \t\n]*([A-Za-z_][A-Za-z0-9_]*)",
            consumer_code.split("#[cfg(test)]", 1)[0],
        ) != [
            "Undefined",
            "Null",
            "Bool",
            "Bool",
            "Int",
            "Float64Bits",
            "BigIntI32",
            "BigIntBytes",
            "EmptyString",
            "ConstantString",
            "AtomString",
            "IntegerAtomString",
        ]
        or " ".join(scalar_string_code.split())
        != " ".join(rust_code_only(expected_scalar_string_source).split())
    ):
        fail(
            "binary-object-consumer-scalar-mapping",
            f"{consumer_relative} must retain the reviewed primitive, BigInt, and UTF-16 String provenance mapping",
        )
    if (
        " ".join(scalar_constant_code.split())
        != " ".join(rust_code_only(expected_scalar_constant_source).split())
        or " ".join(bigint_lowering_code.split())
        != " ".join(rust_code_only(expected_bigint_lowering_source).split())
        or " ".join(bigint_decoder_code.split())
        != " ".join(rust_code_only(expected_bigint_decoder_source).split())
        or len(re.findall(r"\blower_bigint_constant\b", consumer_code)) != 2
        or len(re.findall(r"\bdecode_bigint_constant\b", consumer_code)) != 3
        or len(re.findall(r"\bJsBigInt[ \t\n]*::[ \t\n]*decode_bc5_signed_le\b", consumer_code)) != 1
    ):
        fail(
            "binary-object-consumer-bigint",
            f"{consumer_relative} must decode BigIntBytes exactly once through the unique canonical BigInt helper and lower all primitive pushes through reviewed helpers",
        )

    ordinary_detached_lowering, _, _ = unique_braced_item(
        consumer_production_code,
        re.compile(r"\bfn[ \t\n]+lower_detached_primitive\b[^{};]*\{"),
        "ordinary-leaf-consumer-lowering",
        "detached primitive lowering",
    )
    ordinary_instruction_lowering, _, _ = unique_braced_item(
        consumer_production_code,
        re.compile(r"\bfn[ \t\n]+lower_ordinary_leaf_op\b[^{};]*\{"),
        "ordinary-leaf-consumer-lowering",
        "typed instruction lowering",
    )
    require_normalized_code_sha256(
        "ordinary-leaf-consumer-lowering",
        "lower_ordinary_leaf_op must remain one alias-free exhaustive typed publisher match",
        ordinary_instruction_lowering,
        "97744dba5220743068ed1afe133603daaf32baf362b361dbb4d9d4efdda4f6c8",
    )
    detached_variants = re.findall(
        r"\bDetachedPrimitive[ \t\n]*::[ \t\n]*(\w+)",
        ordinary_detached_lowering,
    )
    normalized_detached_lowering = " ".join(ordinary_detached_lowering.split())
    if detached_variants != [
        "Undefined", "Null", "Bool", "Int", "Float64Bits", "String",
        "BigIntSignedLeCanonical",
    ] or any(
        normalized_detached_lowering.count(fragment) != 1
        for fragment in (
            "DetachedPrimitive::Float64Bits(bits) => Value::Float(f64::from_bits(bits))",
            "DetachedPrimitive::String(units) => Value::String( JsString::try_from_utf16(units.into_vec())",
            "DetachedPrimitive::BigIntSignedLeCanonical(bytes) => { Value::BigInt(decode_bigint_constant(&bytes)?) }",
            "lower_primitive_constant(value)",
        )
    ):
        fail(
            "ordinary-leaf-consumer-lowering",
            "detached primitives must retain bit-preserving Float64, canonical BigInt, UTF-16 String, and primitive publication",
        )
    published_variants = re.findall(
        r"\bOrdinaryLeafOp[ \t\n]*::[ \t\n]*(\w+)",
        ordinary_instruction_lowering,
    )
    expected_published_variants = """
        Nop Object ToObject PushThis PushI32 PushConst PushUndefined PushNull PushBool PushBool PushBigIntI32
        PushEmptyString Stack Unary PostDec PostInc GetLocal PutLocal SetLocal
        GetArgument PutArgument SetArgument Binary Predicate IfFalse IfTrue Goto
        Call TailCall Construct CallMethod TailCallMethod ArrayFrom Apply Return
        ReturnUndefined Throw ThrowReadOnly
    """.split()
    found_publisher_arms = rustfmt_match_arms(
        ordinary_instruction_lowering, "OrdinaryLeafOp::"
    )
    publisher_families = {
        "Stack": ("Drop", "Nip", "Dup", "Dup1", "Dup3", "Insert2", "Insert3", "Insert4", "Perm3", "Perm4", "Perm5", "Swap", "Rot4Left"),
        "Unary": ("Neg", "Plus", "Dec", "Inc", "BitNot", "LogicalNot", "TypeOf"),
        "Binary": ("Add", "Sub", "Mul", "Div", "Mod", "Pow", "Shl", "Sar", "Shr", "LessThan", "LessThanOrEqual", "GreaterThan", "GreaterThanOrEqual", "Equal", "NotEqual", "StrictEqual", "StrictNotEqual", "BitAnd", "BitXor", "BitOr"),
        "Predicate": ("IsUndefinedOrNull", "IsUndefined", "IsNull", "TypeOfIsUndefined", "TypeOfIsFunction"),
    }
    engine_renames = {
        "LogicalNot": "Not", "LessThan": "Lt", "LessThanOrEqual": "Lte",
        "GreaterThan": "Gt", "GreaterThanOrEqual": "Gte", "Equal": "Eq",
        "NotEqual": "Neq", "StrictEqual": "StrictEq", "StrictNotEqual": "StrictNeq",
    }
    publisher_arm_map = dict(found_publisher_arms)
    for family, variants in publisher_families.items():
        expected_body = "match operation { " + " ".join(
            f"OrdinaryLeaf{family}Op::{variant} => "
            f"Instruction::{engine_renames.get(variant, variant)},"
            for variant in variants
        ) + " }"
        if publisher_arm_map.get(f"OrdinaryLeafOp::{family}(operation)") != expected_body:
            fail(
                "ordinary-leaf-consumer-lowering",
                f"the {family.lower()} publisher mapping drifted",
            )
    publisher_direct_rows = """
OrdinaryLeafOp::Nop @ Instruction::Nop
OrdinaryLeafOp::Object @ Instruction::Object
OrdinaryLeafOp::ToObject @ Instruction::ToObject
OrdinaryLeafOp::PushThis @ Instruction::PushThis
OrdinaryLeafOp::PushI32(value) @ Instruction::PushI32(value)
OrdinaryLeafOp::PushConst(index) @ Instruction::PushConst(index)
OrdinaryLeafOp::PushUndefined @ Instruction::Undefined
OrdinaryLeafOp::PushNull @ Instruction::Null
OrdinaryLeafOp::PushBool(false) @ Instruction::PushFalse
OrdinaryLeafOp::PushBool(true) @ Instruction::PushTrue
OrdinaryLeafOp::PostDec @ Instruction::PostDec
OrdinaryLeafOp::PostInc @ Instruction::PostInc
OrdinaryLeafOp::GetLocal(index) @ Instruction::GetLocal(index)
OrdinaryLeafOp::PutLocal(index) @ Instruction::PutLocal(index)
OrdinaryLeafOp::SetLocal(index) @ Instruction::SetLocal(index)
OrdinaryLeafOp::GetArgument(index) @ Instruction::GetArg(index)
OrdinaryLeafOp::PutArgument(index) @ Instruction::PutArg(index)
OrdinaryLeafOp::SetArgument(index) @ Instruction::SetArg(index)
OrdinaryLeafOp::IfFalse(target) @ Instruction::IfFalse(target)
OrdinaryLeafOp::IfTrue(target) @ Instruction::IfTrue(target)
OrdinaryLeafOp::Goto(target) @ Instruction::Goto(target)
OrdinaryLeafOp::Call(argument_count) @ Instruction::Call(argument_count)
OrdinaryLeafOp::TailCall(argument_count) @ Instruction::TailCall(argument_count)
OrdinaryLeafOp::Construct(argument_count) @ Instruction::Construct(argument_count)
OrdinaryLeafOp::CallMethod(argument_count) @ Instruction::CallMethod(argument_count)
OrdinaryLeafOp::TailCallMethod(argument_count) @ { Instruction::TailCallMethod(argument_count) }
OrdinaryLeafOp::ArrayFrom(element_count) @ Instruction::ArrayFrom(element_count)
OrdinaryLeafOp::Apply(kind) @ Instruction::Apply(match kind { OrdinaryLeafApplyKind::Call => ApplyKind::Call, OrdinaryLeafApplyKind::Construct => ApplyKind::Construct, })
OrdinaryLeafOp::Return @ Instruction::Return
OrdinaryLeafOp::ReturnUndefined @ Instruction::ReturnUndefined
OrdinaryLeafOp::Throw @ Instruction::Throw
""".strip().splitlines()
    excluded_publisher_arms = {
        *(f"OrdinaryLeafOp::{family}(operation)" for family in publisher_families),
        "OrdinaryLeafOp::PushBigIntI32(_) | OrdinaryLeafOp::PushEmptyString",
        "OrdinaryLeafOp::ThrowReadOnly(_)",
    }
    found_publisher_direct = [
        arm for arm in found_publisher_arms if arm[0] not in excluded_publisher_arms
    ]
    expected_publisher_direct = [
        tuple(row.split(" @ ", 1)) for row in publisher_direct_rows
    ]
    normalized_instruction_lowering = " ".join(ordinary_instruction_lowering.split())
    synthetic_index_arm, _, _ = unique_braced_item(
        ordinary_instruction_lowering,
        re.compile(
            r"OrdinaryLeafOp[ \t\n]*::[ \t\n]*PushBigIntI32[ \t\n]*\([^)]*\)"
            r"[ \t\n]*\|[ \t\n]*OrdinaryLeafOp[ \t\n]*::[ \t\n]*PushEmptyString"
            r"[ \t\n]*=>[ \t\n]*\{"
        ),
        "ordinary-leaf-consumer-lowering",
        "synthetic constant index arm",
    )
    normalized_synthetic_index = " ".join(synthetic_index_arm.split())
    throw_read_only_publisher_arm, _, _ = unique_braced_item(
        ordinary_instruction_lowering,
        re.compile(
            r"OrdinaryLeafOp[ \t\n]*::[ \t\n]*ThrowReadOnly"
            r"[ \t\n]*\([^)]*\)[ \t\n]*=>[ \t\n]*\{"
        ),
        "ordinary-leaf-consumer-lowering",
        "ThrowReadOnly synthetic constant index arm",
    )
    normalized_throw_read_only_publisher = " ".join(
        throw_read_only_publisher_arm.split()
    )
    synthetic_index_fragments = (
        "let index = *next_synthetic_index;",
        "*next_synthetic_index = next_synthetic_index.checked_add(1)",
        "Instruction::PushConst(index)",
    )
    synthetic_index_offsets = [
        normalized_synthetic_index.find(fragment) for fragment in synthetic_index_fragments
    ]
    if (
        len(found_publisher_arms) != 37
        or found_publisher_direct != expected_publisher_direct
        or published_variants != expected_published_variants
        or any(
        normalized_instruction_lowering.count(fragment) != 1
        for fragment in (
            "OrdinaryLeafOp::PushBigIntI32(_) | OrdinaryLeafOp::PushEmptyString =>",
        )
        )
        or normalized_instruction_lowering.count("Instruction::PushConst(index)") != 2
        or normalized_throw_read_only_publisher != (
            "OrdinaryLeafOp::ThrowReadOnly(_) => { "
            "let index = *next_synthetic_index; "
            "*next_synthetic_index = next_synthetic_index.checked_add(1).ok_or_else(|| { "
            "RuntimeError::Engine(Error::internal( , )) })?; "
            "Instruction::ThrowReadOnly(index) }"
        )
        or (
            any(offset < 0 for offset in synthetic_index_offsets)
            or synthetic_index_offsets != sorted(synthetic_index_offsets)
            or any(
                normalized_synthetic_index.count(fragment) != 1
                for fragment in synthetic_index_fragments
            )
        )
    ):
        fail(
            "ordinary-leaf-consumer-lowering",
            "ordinary operations must retain their complete typed publisher mapping, one-for-one Nop/Object/ToObject/PushThis publication, and stable synthetic indices",
        )

    ordinary_consumer_seals = (
        ("typed-verifier error classification", "map_ordinary_leaf_verification_error", "43d7c7fd3f77c74c9a4ba88cfa14f7d7e8394f7be1100214e8b08a424a8b59ee"),
        ("archive error classification", "map_ordinary_leaf_read_error", "5bb4c99694272a03044d353e48e3f3848ade1e1bfbad2b87a552e1bafa45ba74"),
    )
    for description, function_name, expected_hash in ordinary_consumer_seals:
        item_code, _, _ = unique_braced_item(
            consumer_production_code,
            re.compile(
                rf"(?m)^[ \t]*(?:const[ \t\n]+)?fn[ \t\n]+"
                rf"{function_name}\b[^{{}};]*\{{"
            ),
            "ordinary-leaf-consumer-lowering",
            description,
        )
        if item_code and normalized_code_sha256(item_code) != expected_hash:
            fail(
                "ordinary-leaf-consumer-lowering",
                f"ordinary-leaf {description} drifted from its reviewed normalized implementation",
            )

    consumer_raw_archive_dependency = re.search(
        r"\b(?:BytecodeImage|ImageCode|ImageInstructionSpan|ImageRelocation|"
        r"NativeCodePlan|NativeOperands|NativeInstruction|FunctionId|ImageAtom|"
        r"PinnedAtomId)\b|\.[ \t\n]*(?:as_bytes|atom_relocations)[ \t\n]*\(",
        consumer_production_code,
    )
    if consumer_raw_archive_dependency is not None:
        fail(
            "ordinary-leaf-consumer-import",
            "the runtime publisher may consume only owned ordinary-leaf DTOs, never archival images, raw code, native-plan operands, identities, or sidecars; found "
            + location(
                consumer_relative,
                consumer_source,
                consumer_raw_archive_dependency.start(),
            ),
        )

    consumer_special_case_pattern = re.compile(
        r"(?i:\btest262\b|\bfixture(?:_[A-Za-z0-9_]+)?\b|"
        r"\b(?:source|input|bytes)_[A-Za-z0-9_]*(?:hash|digest|sha_?(?:1|256|512))\b)|"
        r"\bbytes[ \t\n]*(?:\.[A-Za-z_][A-Za-z0-9_]*[ \t\n]*\([^;\n]*\))*"
        r"\.[ \t\n]*(?:contains|starts_with|ends_with|windows)[ \t\n]*\(",
    )
    consumer_special_case = consumer_special_case_pattern.search(
        consumer_source.split("#[cfg(test)]", 1)[0]
    )
    if consumer_special_case is not None:
        fail(
            "ordinary-leaf-consumer-special-casing",
            "the ordinary-leaf publisher must not dispatch on Test262, fixture, digest, or exact input-byte identity; found "
            + location(
                consumer_relative,
                consumer_source,
                consumer_special_case.start(),
            ),
        )

    for match in re.finditer(r"\b(?:r#)?number[ \t\n]*\(", consumer_code):
        fail(
            "binary-object-consumer-float64",
            f"{consumer_relative} must not normalize an authenticated Float64 tag through Value::number or an alias; found "
            + location(consumer_relative, consumer_source, match.start()),
        )

    if (
        len(re.findall(r"\bUnlinkedConstant[ \t\n]*::[ \t\n]*atom_string[ \t\n]*\(", consumer_code)) != 3
        or len(re.findall(r"\batom_string\b", consumer_code)) != 3
    ):
        fail(
            "binary-object-consumer-atom-string",
            "only scalar direct/ordinary atom Strings and the ordinary direct-empty synthetic constant may use the atom-string publication marker",
        )

    consumer_forbidden_patterns = (
        (
            "binary-object-consumer-bigint-eager-negation",
            re.compile(
                r"\b(?:std|core)[ \t\n]*::[ \t\n]*ops[ \t\n]*::"
                r"[ \t\n]*Neg\b|"
                r"\bJsBigInt[ \t\n]*::[ \t\n]*(?:neg|negate)\b|"
                r"\b(?:value|bigint)[ \t\n]*\.[ \t\n]*"
                r"(?:neg|negate|checked_neg|wrapping_neg)[ \t\n]*\(|"
                r"-[ \t\n]*(?:value|bigint)\b"
            ),
            "eager BigInt negation; authenticated unary negation must remain Instruction::Neg execution semantics",
        ),
        (
            "binary-object-consumer-alternate-entrypoint",
            re.compile(
                r"\b(?:(?:self|runtime)[ \t\n]*\.[ \t\n]*|Runtime[ \t\n]*::[ \t\n]*)"
                r"(?!(?:publish_unlinked_function|publish_verified_unlinked_function)\b)"
                r"(?:compile|publish)_[A-Za-z_][A-Za-z0-9_]*[ \t\n]*\("
            ),
            "an alternate runtime compilation or publication entry point",
        ),
        (
            "binary-object-consumer-heap-type",
            re.compile(
                r"\b(?:FunctionBytecodeData|BytecodeConstant|FunctionBytecodeId|ObjectId|"
                r"RawValue|Heap|HeapObject|ObjectRef)\b"
            ),
            "a direct runtime heap representation type",
        ),
        (
            "binary-object-consumer-root-forge",
            re.compile(
                r"\bFunctionBytecodeRef[ \t\n]*\{|\bFunctionBytecodeRef[ \t\n]*::"
                r"[ \t\n]*(?:from_owned_handle|from_borrowed_handle)\b"
            ),
            "a direct FunctionBytecodeRef constructor",
        ),
        (
            "binary-object-consumer-atom-interning",
            re.compile(r"\bintern(?:_[A-Za-z0-9_]+)?\b"),
            "an atom-interning identifier",
        ),
        (
            "binary-object-consumer-vm-dependency",
            re.compile(
                r"\bcrate[ \t\n]*::[ \t\n]*(?:r#)?vm\b|"
                r"\b(?:super[ \t\n]*::[ \t\n]*)+(?:r#)?vm\b"
            ),
            "the VM module",
        ),
        (
            "binary-object-consumer-compiler-dependency",
            re.compile(
                r"\bcrate[ \t\n]*::[ \t\n]*(?:r#)?compiler\b|"
                r"\b(?:super[ \t\n]*::[ \t\n]*)+(?:r#)?compiler\b"
            ),
            "the source compiler",
        ),
        (
            "binary-object-consumer-unsafe",
            re.compile(r"\bunsafe\b|\bNonNull\b|\*[ \t\n]*(?:const|mut)\b"),
            "unsafe code or native pointers",
        ),
    )
    for code_name, pattern, description in consumer_forbidden_patterns:
        for match in pattern.finditer(consumer_code):
            fail(
                code_name,
                f"{consumer_relative} must not use {description}; found "
                + location(consumer_relative, consumer_source, match.start()),
            )

bytecode_publish_relative = "src/runtime/bytecode_publish.rs"
bytecode_publish_source = read_source(bytecode_publish_relative)
bytecode_publish_code = rust_code_only(bytecode_publish_source)
if (
    len(
        re.findall(
            r"(?m)^[ \t]*TrustedOrdinaryLeaf[ \t]*,[ \t]*$",
            bytecode_publish_code,
        )
    )
    != 1
    or len(re.findall(r"\bTrustedOrdinaryLeaf\b", bytecode_publish_code)) != 8
):
    fail(
        "ordinary-leaf-verifier-role",
        "RootPublication must declare one distinct TrustedOrdinaryLeaf role with the reviewed generic-verifier use set",
    )

ordinary_verifier_entry_code, _, _ = unique_braced_item(
    bytecode_publish_code,
    re.compile(
        r"(?m)^[ \t]*pub[ \t\n]*\([ \t\n]*in[ \t\n]+crate[ \t\n]*::"
        r"[ \t\n]*runtime[ \t\n]*\)[ \t\n]+fn[ \t\n]+"
        r"verify_unlinked_ordinary_leaf\b[^{};]*\{"
    ),
    "ordinary-leaf-verifier-entrypoint",
    "dedicated ordinary-leaf verifier entry point",
)
expected_ordinary_verifier_entry = "pub(in crate::runtime) fn verify_unlinked_ordinary_leaf( function: &UnlinkedFunction, ) -> Result<(), RuntimeError> { verify_unlinked_tree_with_root(function, RootPublication::TrustedOrdinaryLeaf) }"
if " ".join(ordinary_verifier_entry_code.split()) != " ".join(
    expected_ordinary_verifier_entry.split()
):
    fail(
        "ordinary-leaf-verifier-entrypoint",
        "verify_unlinked_ordinary_leaf must enter the generic verifier through only the distinct TrustedOrdinaryLeaf role",
    )

ordinary_verifier_arm_pattern = re.compile(
    r"RootPublication[ \t\n]*::[ \t\n]*TrustedOrdinaryLeaf"
    r"[ \t\n]*=>[ \t\n]*\{"
)
ordinary_verifier_arms = []
for arm_match in ordinary_verifier_arm_pattern.finditer(bytecode_publish_code):
    arm_code, _, _ = braced_item_from_match(
        bytecode_publish_code,
        arm_match,
        "ordinary-leaf-verifier-role",
        "TrustedOrdinaryLeaf verifier arm",
    )
    ordinary_verifier_arms.append(arm_code)
expected_ordinary_closure_arm = 'RootPublication::TrustedOrdinaryLeaf => { return Err(RuntimeError::Engine(Error::internal("trusted ordinary leaf retained a closure descriptor", ))); }'
if (
    len(ordinary_verifier_arms) != 2
    or normalized_code_sha256(ordinary_verifier_arms[0])
        != "78509ea6396da4c20a0ee2b34304ac0224552e20e3b89e4346ab8173810c8c7e"
    or " ".join(ordinary_verifier_arms[1].split())
    != " ".join(rust_code_only(expected_ordinary_closure_arm).split())
):
    fail(
        "ordinary-leaf-verifier-role",
        "the dedicated verifier must retain its exact fail-closed metadata/debug/parameter/local/primitive-only checks and closure rejection; "
        f"found {len(ordinary_verifier_arms)} role arms",
    )

function_relative = "src/function.rs"
function_source = read_source(function_relative)
function_code = rust_code_only(function_source)
plain_primitive_code, _, _ = unique_braced_item(
    function_code,
    re.compile(
        r"(?m)^[ \t]*pub[ \t\n]*\([ \t\n]*crate[ \t\n]*\)[ \t\n]+const"
        r"[ \t\n]+fn[ \t\n]+is_plain_primitive\b[^{};]*\{"
    ),
    "ordinary-leaf-plain-primitive",
    "UnlinkedConstant plain-primitive discriminator",
)
expected_plain_primitive = "pub(crate) const fn is_plain_primitive(&self) -> bool { matches!(self.0, UnlinkedConstantKind::Primitive(_)) }"
if " ".join(plain_primitive_code.split()) != " ".join(expected_plain_primitive.split()):
    fail(
        "ordinary-leaf-plain-primitive",
        "ordinary-leaf verification must classify only UnlinkedConstantKind::Primitive as a plain primitive",
    )
empty_atom_code, _, _ = unique_braced_item(
    function_code,
    re.compile(
        r"(?m)^[ \t]*pub[ \t\n]*\([ \t\n]*crate[ \t\n]*\)[ \t\n]+fn"
        r"[ \t\n]+is_empty_atom_string\b[^{};]*\{"
    ),
    "ordinary-leaf-plain-primitive",
    "exact empty atom-String discriminator",
)
expected_empty_atom = "pub(crate) fn is_empty_atom_string(&self) -> bool { matches!( &self.0, UnlinkedConstantKind::AtomString(Value::String(value)) if value.is_empty() ) }"
if " ".join(empty_atom_code.split()) != expected_empty_atom:
    fail(
        "ordinary-leaf-plain-primitive",
        "ordinary-leaf verification may admit only the exact empty atom String beside plain primitives",
    )

context_relative = "src/runtime/context.rs"
context_source = read_source(context_relative)
context_code = rust_code_only(context_source)
ordinary_public_api_code, _, _ = unique_braced_item(
    context_code,
    re.compile(
        r"(?m)^[ \t]*pub[ \t\n]+fn[ \t\n]+read_trusted_ordinary_function"
        r"\b[^{};]*\{"
    ),
    "ordinary-leaf-public-api",
    "Context ordinary-leaf public API",
)
expected_ordinary_public_api = "pub fn read_trusted_ordinary_function( &mut self, bytes: &[u8], root_constant_index: u32, ) -> Result<CallableRef, RuntimeError> { let result = self.runtime.read_trusted_ordinary_function_in_realm( self.realm, bytes, root_constant_index, ); self.finish_trusted_bytecode_read(result) }"
if " ".join(ordinary_public_api_code.split()) != " ".join(
    expected_ordinary_public_api.split()
):
    fail(
        "ordinary-leaf-public-api",
        "Context::read_trusted_ordinary_function must retain its exact selector, realm bridge, and trusted-read error finishing flow",
    )
trusted_read_finish_code, _, _ = unique_braced_item(
    context_code,
    re.compile(
        r"(?m)^[ \t]*fn[ \t\n]+finish_trusted_bytecode_read\b[^{};]*\{"
    ),
    "ordinary-leaf-public-api",
    "shared trusted-bytecode read finisher",
)
if trusted_read_finish_code and normalized_code_sha256(
    trusted_read_finish_code
) != "398f8677cddce39a30934cca10dfcde5fef30279c69f56e1f83f937aee7f745e":
    fail(
        "ordinary-leaf-public-api",
        "trusted bytecode reads must convert only JavaScript-visible errors into pending exceptions and preserve Unsupported/Internal directly",
    )

bytecode_source = read_source("src/bytecode.rs")
bytecode_code = rust_code_only(bytecode_source)
bytecode_production_code = bytecode_code.split("#[cfg(test)]\nmod tests", 1)[0]
vm_code = rust_code_only(read_source("src/vm.rs"))
value_code = rust_code_only(read_source("src/value.rs"))
atom_code = rust_code_only(read_source("src/atom.rs"))
engine_string_fragments = (
    (bytecode_code, "PushAtomValueIndex(u32),"),
    (bytecode_code, "Self::PushI32(_) | Self::PushAtomValueIndex(_) | Self::PushConst(_)"),
    (bytecode_code, "Instruction::PushAtomValueIndex(index) if *index > crate::atom::ATOM_MAX_INT"),
    (vm_code, "Instruction::PushAtomValueIndex(value) => self.stack.push(Value::String( crate::value::JsString::from_fresh_decimal_u32(*value), ))"),
    (atom_code, "AtomSpelling::Integer(value) => Ok(JsString::from_fresh_decimal_u32(value))"),
    (value_code, "pub(crate) fn from_fresh_decimal_u32(mut value: u32) -> Self"),
    (value_code, "digits[start] = b'0' + (value % 10) as u8;"),
    (value_code, "Self(Rc::new(StringRepr::Latin1( digits[start..].to_vec().into_boxed_slice(), )))"),
)
if any(" ".join(code.split()).count(fragment) != 1 for code, fragment in engine_string_fragments):
    fail(
        "scalar-string-engine-path",
        "tagged integer atoms must verify within JS_ATOM_MAX_INT and execute through one fresh narrow String instruction",
    )

normalized_bytecode = " ".join(bytecode_code.split())
if normalized_bytecode.count("| Self::ReturnUndefined | Self::ThrowRedeclaration(_)" ) != 1:
    fail(
        "ordinary-leaf-engine-semantics",
        "ReturnUndefined must remain a zero-pop, zero-push terminal instruction",
    )
instruction_code, _, _ = unique_braced_item(
    bytecode_production_code,
    re.compile(r"\bpub[ \t\n]+enum[ \t\n]+Instruction[ \t\n]*\{"),
    "stage3c-instruction-shape",
    "engine Instruction enum",
)
tail_instruction_payloads = {
    name: [
        " ".join(payload.split())
        for payload in re.findall(
            rf"\b{name}[ \t\n]*\(([^()]*)\)[ \t\n]*,", instruction_code
        )
    ]
    for name in ("TailCall", "TailCallMethod")
}
if tail_instruction_payloads != {"TailCall": ["u16"], "TailCallMethod": ["u16"]}:
    fail(
        "stage3c-instruction-shape",
        "Instruction must retain distinct TailCall(u16) and TailCallMethod(u16) terminal payloads; "
        f"found {tail_instruction_payloads}",
    )
if (
    enum_variant_names(instruction_code).count("Throw") != 1
    or re.search(r"\bThrow[ \t\n]*[({]", instruction_code)
):
    fail(
        "stage3d-instruction-shape",
        "Instruction must retain exactly one operand-free Throw completion",
    )
predicate_requirements = {
    "IsUndefinedOrNull": ("matches!(value, Value::Undefined | Value::Null)", False),
    "IsUndefined": ("matches!(value, Value::Undefined)", False),
    "IsNull": ("matches!(value, Value::Null)", False),
    "TypeOfIsUndefined": ("matches!(value, Value::Undefined) || host.is_html_dda(&value)?", True),
    "TypeOfIsFunction": ("!host.is_html_dda(&value)? && host.is_callable(&value)?", True),
}
for instruction, (required, uses_html_dda) in predicate_requirements.items():
    arm, _, _ = unique_braced_item(
        vm_code,
        re.compile(rf"\bInstruction[ \t\n]*::[ \t\n]*{instruction}[ \t\n]*=>[ \t\n]*\{{"),
        "ordinary-leaf-engine-semantics",
        f"VM {instruction} arm",
    )
    normalized_arm = " ".join(arm.split())
    if (
        normalized_arm.count("let value = self.pop()?;") != 1
        or normalized_arm.count(required) != 1
        or ("host.is_html_dda" in normalized_arm) != uses_html_dda
    ):
        fail(
            "ordinary-leaf-engine-semantics",
            f"VM {instruction} must retain its exact QuickJS tag/HTMLDDA predicate",
        )
return_undefined_arm, _, _ = unique_braced_item(
    vm_code,
    re.compile(r"\bInstruction[ \t\n]*::[ \t\n]*ReturnUndefined[ \t\n]*=>[ \t\n]*\{"),
    "ordinary-leaf-engine-semantics",
    "VM ReturnUndefined arm",
)
if " ".join(return_undefined_arm.split()).count(
    "return Ok(Some(Completion::Return(Value::Undefined)));"
) != 1:
    fail(
        "ordinary-leaf-engine-semantics",
        "ReturnUndefined must complete directly with undefined without reading the operand stack",
    )

# The self-test fixture exercises codec isolation with deliberately tiny engine
# stubs. Full-source mutation canaries and the real tree carry no marker and
# therefore must satisfy every Stage3B runtime invariant below.
if not (root / ".boundary-self-test").is_file():
    stage3b_sources = {
        "src/runtime.rs": runtime_code,
        "src/vm.rs": vm_code,
        "src/bytecode.rs": bytecode_code,
        "src/runtime/context.rs": context_code,
    }
    stage3b_items: dict[tuple[str, str], str] = {}

    def stage3b_code(relative: str) -> str:
        if relative not in stage3b_sources:
            stage3b_sources[relative] = rust_code_only(read_source(relative))
        return stage3b_sources[relative]

    def stage3b_function(relative: str, name: str, diagnostic: str) -> str:
        key = (relative, name)
        if key not in stage3b_items:
            stage3b_items[key] = unique_braced_item(
                stage3b_code(relative),
                re.compile(rf"\bfn[ \t\n]+{re.escape(name)}\b[^{{}};]*\{{"),
                diagnostic,
                f"{relative}::{name}",
            )[0]
        return stage3b_items[key]

    if len(re.findall(
        r"\bstruct[ \t\n]+ConstructorRef[ \t\n]*\([ \t\n]*ObjectRef[ \t\n]*\)[ \t\n]*;",
        runtime_code,
    )) != 1:
        fail("stage3b-constructor-capability", "ConstructorRef must remain the private [[Construct]] capability")

    construct_new_target_code = unique_braced_item(
        runtime_code,
        re.compile(r"\benum[ \t\n]+ConstructNewTarget[ \t\n]*\{"),
        "stage3b-new-target-capability",
        "validated-or-raw newTarget carrier",
    )[0]
    construct_new_target_payloads = {
        name: [" ".join(value.split()) for value in re.findall(
            rf"\b{name}[ \t\n]*\(([^()]*)\)[ \t\n]*,", construct_new_target_code
        )]
        for name in ("Validated", "Raw")
    }
    if (
        enum_variant_names(construct_new_target_code) != ["Validated", "Raw"]
        or construct_new_target_payloads != {"Validated": ["ConstructorRef"], "Raw": ["Value"]}
    ):
        fail(
            "stage3b-new-target-capability",
            f"ConstructNewTarget must remain Validated(ConstructorRef) or Raw(Value); found {construct_new_target_payloads}",
        )

    constructor_from_value = stage3b_function(
        "src/runtime.rs", "constructor_from_value", "stage3b-constructor-capability"
    )
    normalized_constructor_from_value = " ".join(constructor_from_value.split())
    if (
        "Result<NativeConversion<ConstructorRef>, RuntimeError>" not in normalized_constructor_from_value
        or normalized_constructor_from_value.count("object_data.is_constructor") != 1
        or normalized_constructor_from_value.count("ConstructorRef::from_validated_object(object)") != 1
        or re.search(r"\b(?:CallableRef|is_callable|as_callable|callable_from_value)\b", constructor_from_value)
    ):
        fail(
            "stage3b-constructor-capability",
            "constructor conversion must validate only [[Construct]], without CallableRef narrowing",
        )

    vm_apply_arm = unique_braced_item(
        vm_code,
        re.compile(
            r"\bInstruction[ \t\n]*::[ \t\n]*Apply[ \t\n]*\([ \t\n]*kind"
            r"[ \t\n]*\)[ \t\n]*=>[ \t\n]*\{"
        ),
        "stage3b-apply-stack",
        "VM Apply dispatch arm",
    )[0]
    require_ordered_fragments(
        "stage3b-apply-stack",
        "Apply must pop list, receiver-or-newTarget, and function before one typed host call",
        vm_apply_arm,
        (
            "let argument_array = self.pop()?;",
            "let this_or_new_target = self.pop()?;",
            "let function = self.pop()?;",
            "host.apply(function, this_or_new_target, argument_array, *kind)?",
        ),
    )
    if bytecode_code.count("Self::Apply(_) | Self::ApplySuper => (3, 1),") != 1:
        fail("stage3b-apply-stack", "Apply must retain its exact three-pop/one-push verifier effect")

    tail_stack_effects = (
        "Self::TailCall(argument_count) => (*argument_count as usize + 1, 0),",
        "Self::TailCallMethod(argument_count) => (*argument_count as usize + 2, 0),",
    )
    stack_effect_item = stage3b_function(
        "src/bytecode.rs", "stack_effect", "stage3c-tail-verifier"
    )
    require_normalized_code_sha256(
        "stage3c-tail-verifier",
        "Instruction::stack_effect must remain the reviewed alias-free exhaustive stack model",
        stack_effect_item,
        "a6b0111cc4ec1e4316e8206d6cd75ccc66d7e910c248015b02de358894454538",
    )
    normalized_stack_effect = " ".join(stack_effect_item.split())
    if any(normalized_stack_effect.count(fragment) != 1 for fragment in tail_stack_effects):
        fail(
            "stage3c-tail-verifier",
            "TailCall and TailCallMethod must preserve argc+1/argc+2 pops and zero pushes",
        )

    verify_parts_item = stage3b_function(
        "src/bytecode.rs", "verify_parts", "stage3c-tail-verifier"
    )
    normalized_verify_parts = " ".join(verify_parts_item.split())
    verifier_terminal_guard = (
        "if matches!( instruction, Instruction::TailCall(_) | "
        "Instruction::TailCallMethod(_) | Instruction::Return | "
        "Instruction::ReturnUndefined | Instruction::ReturnDerived(_) ) && state"
    )
    verifier_terminal_offset = normalized_verify_parts.find(verifier_terminal_guard)
    if verifier_terminal_offset < 0:
        fail(
            "stage3c-tail-verifier",
            "the reviewed terminal verifier corridor is missing",
        )
    else:
        require_normalized_code_sha256(
            "stage3c-tail-verifier",
            "the iterator guard, maximum-depth check, terminal dispatch, and successor enqueue corridor must remain alias-free and ordered",
            normalized_verify_parts[verifier_terminal_offset:],
            "5f18313dcb4ee5192b9fc3a53c76e1fe7e63bfba3eb3e5761033ecf5ad694370",
        )
    tail_terminal_prefix = "Instruction::TailCall(_) | Instruction::TailCallMethod(_)"
    tail_terminal_dispatch = (
        "Instruction::TailCall(_) | Instruction::TailCallMethod(_) | "
        "Instruction::Return | Instruction::ReturnUndefined | "
        "Instruction::ReturnDerived(_) | Instruction::Throw | Instruction::Ret => {}"
    )
    if (
        normalized_verify_parts.count(tail_terminal_prefix) != 2
        or normalized_verify_parts.count(tail_terminal_dispatch) != 1
        or normalized_verify_parts.count("enqueue_target(") != 4
        or normalized_verify_parts.count("enqueue_fallthrough(") != 4
    ):
        fail(
            "stage3c-tail-verifier",
            "both tail invocation instructions must be terminal verifier nodes and must not enqueue fallthrough",
        )

    take_call_arguments_item = stage3b_function(
        "src/vm.rs", "take_call_arguments", "stage3c-tail-vm"
    )
    require_normalized_code_sha256(
        "stage3c-tail-vm",
        "take_call_arguments must retain the exact checked suffix split without argument or fixed-value shadowing",
        take_call_arguments_item,
        "e8f523f68ef01df927a8761c6dc92ddafb0471fadee17ab9ead81f71d50287f4",
    )

    call_dispatch_item = stage3b_function(
        "src/vm.rs", "execute_call_instruction", "stage3c-tail-vm"
    )
    require_normalized_code_sha256(
        "stage3c-tail-vm",
        "execute_call_instruction must retain its alias-free exhaustive call-family dispatch",
        call_dispatch_item,
        "f6b13bdb02ab06e2177beba9cd3bda2f626c6baa97fa09420cbfa2459ef5a97d",
    )
    tail_call_arm = unique_braced_item(
        call_dispatch_item,
        re.compile(
            r"\bInstruction[ \t\n]*::[ \t\n]*TailCall[ \t\n]*\("
            r"[ \t\n]*argument_count[ \t\n]*\)[ \t\n]*=>[ \t\n]*\{"
        ),
        "stage3c-tail-vm",
        "TailCall VM arm",
    )[0]
    tail_method_arm = unique_braced_item(
        call_dispatch_item,
        re.compile(
            r"\bInstruction[ \t\n]*::[ \t\n]*TailCallMethod[ \t\n]*\("
            r"[ \t\n]*argument_count[ \t\n]*\)[ \t\n]*=>[ \t\n]*\{"
        ),
        "stage3c-tail-vm",
        "TailCallMethod VM arm",
    )[0]
    expected_tail_call_arm = (
        "Instruction::TailCall(argument_count) => { "
        "let arguments = self.take_call_arguments(*argument_count, 1)?; "
        "let function = self.pop()?; "
        "return host.call(function, Value::Undefined, arguments).map(Some); }"
    )
    expected_tail_method_arm = (
        "Instruction::TailCallMethod(argument_count) => { "
        "let arguments = self.take_call_arguments(*argument_count, 2)?; "
        "let function = self.pop()?; let receiver = self.pop()?; "
        "return host.call(function, receiver, arguments).map(Some); }"
    )
    if (
        " ".join(tail_call_arm.split()) != expected_tail_call_arm
        or " ".join(tail_method_arm.split()) != expected_tail_method_arm
    ):
        fail(
            "stage3c-tail-vm",
            "tail VM dispatch must preserve undefined/plain and receiver/function/method argument order, then return the host completion directly",
        )

    activation_execute_item = unique_braced_item(
        vm_code,
        re.compile(
            r"\bfn[ \t\n]+execute[ \t\n]*\([^{};]*\)[ \t\n]*->[ \t\n]*"
            r"Result[ \t\n]*<[ \t\n]*Completion[ \t\n]*,[ \t\n]*Error"
            r"[ \t\n]*>[ \t\n]*\{"
        ),
        "stage3c-tail-completion",
        "execute-to-completion activation driver",
    )[0]
    require_normalized_code_sha256(
        "stage3c-tail-completion",
        "the execute-to-completion driver must retain its unique Return terminal and Throw raise flow",
        activation_execute_item,
        "65d316cc1950e983ffc111de71f62e9f9cabb1833acddf93a0980b554df62368",
    )
    require_ordered_fragments(
        "stage3c-tail-completion",
        "tail Return must finish the current frame while Throw still enters the current activation raise path",
        activation_execute_item,
        (
            "Ok(InterpreterExit::Complete(Completion::Return(value))) => { return Ok(Completion::Return(value)); }",
            "Ok(InterpreterExit::Complete(Completion::Throw(value))) => value,",
            "Err(error) if NativeErrorKind::from_javascript_error(error.kind()).is_some() => { host.materialize_error(error)? }",
            "if let Some(completion) = self.raise(raised, host, code.len())? { return Ok(completion); }",
        ),
    )
    activation_run_item = unique_braced_item(
        vm_code,
        re.compile(
            r"\bfn[ \t\n]+run[ \t\n]*\([^{};]*\)[ \t\n]*->[ \t\n]*"
            r"Result[ \t\n]*<[ \t\n]*VmExit[ \t\n]*,[ \t\n]*Error"
            r"[ \t\n]*>[ \t\n]*\{"
        ),
        "stage3c-tail-completion",
        "suspendable activation driver",
    )[0]
    require_normalized_code_sha256(
        "stage3c-tail-completion",
        "the suspendable driver must retain its unique Return terminal and Throw raise flow",
        activation_run_item,
        "53668ef453a43fc676e96f2c6f01c017f3f336df4bba96674562e77361e0f63e",
    )
    raise_item = stage3b_function("src/vm.rs", "raise", "stage3c-tail-completion")
    require_normalized_code_sha256(
        "stage3c-tail-completion",
        "raise must retain one backtrace-first catch/iterator unwind loop without guarded bypasses",
        raise_item,
        "52a25a382122f09433bd29178885bc10c38222aee15f0d5482def7b15b45caab",
    )
    require_ordered_fragments(
        "stage3c-tail-completion",
        "tail Throw must retain backtrace attachment, catch transfer, and iterator unwind on the same activation",
        raise_item,
        (
            "host.ensure_backtrace(&value)?;",
            "let Some(region) = self.regions.pop() else { return Ok(Some(Completion::Throw(value))); };",
            "VmUnwindRegion::Catch { target, stack_depth, } => {",
            "host.prepare_captured_local_reuse()?;",
            "self.stack.truncate(stack_depth);",
            "self.stack.push(value);",
            "self.pc = checked_target(",
            "return Ok(None);",
            "VmUnwindRegion::Iterator { record_base, enabled, .. } => {",
            "match host.iterator_close(iterator, true)? {",
        ),
    )

    execute_inner_item = stage3b_function(
        "src/vm.rs", "execute_inner", "stage3c-tail-vm"
    )
    normalized_execute_inner = " ".join(execute_inner_item.split())
    call_route = (
        "if matches!( instruction, Instruction::Import | Instruction::Call(_) | "
        "Instruction::TailCall(_) | Instruction::Eval { .. } | "
        "Instruction::CallMethod(_) | Instruction::TailCallMethod(_) | "
        "Instruction::Construct(_) | Instruction::ConstructSuper(_) | "
        "Instruction::InitDerivedConstructor | Instruction::Apply(_) | "
        "Instruction::ApplySuper | Instruction::ApplyEval { .. } ) { "
        "if let Some(completion) = self.execute_call_instruction(instruction, host)? { "
        "return Ok(InterpreterExit::Complete(completion)); } continue; }"
    )
    call_route_offset = normalized_execute_inner.find(call_route)
    if call_route_offset < 0 or normalized_execute_inner.count(call_route) != 1:
        fail(
            "stage3c-tail-vm",
            "execute_inner must route the complete call family, including both tail variants, exactly once",
        )
    else:
        call_route_end = call_route_offset + len(call_route)
        require_normalized_code_sha256(
            "stage3c-tail-vm",
            "the execute_inner prefix through call-family routing must not intercept, alias, or remove tail completion",
            normalized_execute_inner[:call_route_end],
            "e425f4d42ef9a3a7b6552994a6f7da31009e70451f5b48e3312581929a54e54c",
        )

    # Raw 48 is admitted by one physical registry row and must stay on the
    # ordinary target all the way through the sanitized instruction DTO. Seal
    # the small accessors and the two translation corridors so an alias cannot
    # preserve the visible Throw arms while bypassing them before publication.
    capability_relative = "src/runtime/binary_object/function_translate/capability.rs"
    capability_code = stage3b_code(capability_relative)
    capability_row_new = stage3b_function(
        capability_relative, "new", "stage3d-throw-capability-route"
    )
    require_normalized_code_sha256(
        "stage3d-throw-capability-route",
        "CapabilityRow::new must preserve the raw, format, and policy fields without rewriting raw48",
        capability_row_new,
        "9e4be6620a97b5136ea400f536be3db0ef7f5cd95c74279dc30969278e19b4fc",
    )
    capability_row_macro = unique_braced_item(
        capability_code,
        re.compile(r"\bmacro_rules[ \t\n]*![ \t\n]*row[ \t\n]*\{"),
        "stage3d-throw-capability-route",
        "capability row macro",
    )[0]
    require_normalized_code_sha256(
        "stage3d-throw-capability-route",
        "the capability row macro must construct the declared audience and recipe directly",
        capability_row_macro,
        "96eac354655d5c9e47c266be4e31d6d1cbfbecc36fe70083bc90e1a59210a59b",
    )
    require_normalized_code_sha256(
        "stage3d-throw-capability-route",
        "row_for must select only the physical raw opcode's registry row",
        stage3b_function(
            capability_relative, "row_for", "stage3d-throw-capability-route"
        ),
        "e05d664bfc0b3bdd581c293111a0a81440e7227aa75b46baab209c061ce3f131",
    )

    dto_relative = "src/runtime/binary_object/function_translate/dto.rs"
    for name, description, expected_hash in (
        (
            "includes_ordinary",
            "ordinary audience membership must remain OrdinaryOnly or Shared",
            "eac985e03d25991731ee08d4964ed8f621886bd9e6d6783dde9f117075346be8",
        ),
        (
            "supports_ordinary",
            "FunctionInstruction must consult its retained audience for ordinary admission",
            "d0f6a6a6678edbe91a19fdf00d9deab39d21d203fd38f57590b4d41abe8d8d8f",
        ),
        (
            "operation",
            "FunctionInstruction must expose the retained typed operation without substitution",
            "903997a7bf00f7594ba961f08a431af59ad1bd42ca678390d39944b8b8c11c0a",
        ),
        (
            "instructions",
            "FunctionCode must expose its retained instruction sequence without filtering",
            "ae989b36159707bc008c1f1b7144b6c0c0b11497ce8b80046157d5a22db99378",
        ),
    ):
        require_normalized_code_sha256(
            "stage3d-throw-translation-route",
            description,
            stage3b_function(dto_relative, name, "stage3d-throw-translation-route"),
            expected_hash,
        )

    translate_relative = "src/runtime/binary_object/function_translate/mod.rs"
    translate_code = stage3b_code(translate_relative)
    require_normalized_code_sha256(
        "stage3d-throw-translation-route",
        "TranslationTarget must delegate ordinary admission to the retained audience",
        stage3b_function(
            translate_relative, "accepts", "stage3d-throw-translation-route"
        ),
        "90579d0e77fd5b936444117085b9b1bb8246364a9f8bc3878c95d7e3e271d16c",
    )
    pending_expansion_impl = unique_braced_item(
        translate_code,
        re.compile(
            r"\bimpl[ \t\n]*<[ \t\n]*'image[ \t\n]*>[ \t\n]+"
            r"PendingExpansion[ \t\n]*<[ \t\n]*'image[ \t\n]*>[ \t\n]*\{"
        ),
        "stage3d-throw-translation-route",
        "PendingExpansion implementation",
    )[0]
    require_normalized_code_sha256(
        "stage3d-throw-translation-route",
        "PendingExpansion must retain each ready operation exactly once and in order",
        pending_expansion_impl,
        "9f43f69140aababa9f85f6845ce991b6133a52693feb7dbb83177e98e3c6001e",
    )
    require_normalized_code_sha256(
        "stage3d-throw-translation-route",
        "operation_for_target must lower admitted ordinary operations and reject outside-target aliases",
        stage3b_function(
            translate_relative,
            "operation_for_target",
            "stage3d-throw-translation-route",
        ),
        "124ecc9366407ebfa14448710b3795c2ee74137aea4f099076fdd13e0b32aec1",
    )
    translate_native_plan_item = stage3b_function(
        translate_relative, "translate_native_plan", "stage3d-throw-translation-route"
    )
    require_normalized_code_sha256(
        "stage3d-throw-translation-route",
        "translate_native_plan must preserve the physical row, target audience, ready operation, and final FunctionCode without a post-lowering remap",
        translate_native_plan_item,
        "fe149677e125ffef44ebac61b8b9799eff3166eafd7efa93e086b84571eb9867",
    )
    require_normalized_corridor_sha256(
        "stage3d-throw-translation-route",
        "translate_native_plan must carry the physical row through its audience expansion into pending code",
        translate_native_plan_item,
        "let row = row_for(opcode);",
        "pending.push(PendingInstruction { audience, diagnostic, expansion, });",
        "de451567beafe6477081e0466f2cdc2b1cb302b8a8202130ef53226509f39524",
    )
    require_normalized_corridor_sha256(
        "stage3d-throw-translation-route",
        "translate_native_plan must publish every ready operation through FunctionInstruction::new without remapping",
        translate_native_plan_item,
        "for instruction in pending {",
        "output.push(FunctionInstruction::new( instruction.audience, instruction.diagnostic, operation, ));",
        "154a0d0ab86cab0cb75aebcc4bb31e8cd6e7b8015e02a4f61dd32eadf17842ae",
    )

    ordinary_relative = "src/runtime/binary_object/ordinary_leaf.rs"
    require_normalized_code_sha256(
        "stage3d-throw-ordinary-route",
        "ordinary lower_code must require ordinary audience support and lower every typed operation once",
        stage3b_function(ordinary_relative, "lower_code", "stage3d-throw-ordinary-route"),
        "802efb137202fe4c4b7380359e627b9e97d676ed0e19b6041a58230e03820557",
    )
    require_normalized_code_sha256(
        "stage3d-throw-ordinary-route",
        "OrdinaryLeafDraft::into_parts must retain metadata, constants, and code without substitution",
        stage3b_function(ordinary_relative, "into_parts", "stage3d-throw-ordinary-route"),
        "cb258f4d808ff5458a2be60fe0050ebc734736ec811b2232f9ff6c031e7e1cf9",
    )
    require_normalized_code_sha256(
        "stage3e-read-only-ordinary-route",
        "admit_image must pass the authenticated input-atom slot count into the typed ordinary lowering without bypass",
        stage3b_function(ordinary_relative, "admit_image", "stage3e-read-only-ordinary-route"),
        "afec36c89046ac4ddb822a58c95bb6d1f8683ce6cedfea0b2718c4d9bf4434ae",
    )
    input_atom_ledger_impl = unique_braced_item(
        stage3b_code(ordinary_relative),
        re.compile(r"\bimpl[ \t\n]+InputAtomLedger[ \t\n]*\{"),
        "stage3e-read-only-atom-ledger",
        "InputAtomLedger implementation",
    )[0]
    require_normalized_code_sha256(
        "stage3e-read-only-atom-ledger",
        "the input-atom ledger must admit at most one slot, require any declared slot to be consumed by raw49, and preserve provenance",
        input_atom_ledger_impl,
        "3b2000fa311f9de95eca3794884907f8b565f2cad4f52b2a2ce0369226799057",
    )
    detached_atom_name_impl = unique_braced_item(
        stage3b_code(ordinary_relative),
        re.compile(r"\bimpl[ \t\n]+DetachedAtomName[ \t\n]*\{"),
        "stage3e-read-only-atom-ledger",
        "DetachedAtomName implementation",
    )[0]
    require_normalized_code_sha256(
        "stage3e-read-only-atom-ledger",
        "DetachedAtomName must release only its owned UTF-16 units into publication",
        detached_atom_name_impl,
        "fd37c0c33e81d7c70073a4b7ceececc409aaad32680ab4384f294f49507147b9",
    )
    require_normalized_code_sha256(
        "stage3e-read-only-atom-ledger",
        "copy_read_only_name must admit only String atoms and preserve every UTF-16 unit in an owned payload",
        stage3b_function(ordinary_relative, "copy_read_only_name", "stage3e-read-only-atom-ledger"),
        "b5f7bc69b10a77a23b94db94558816a972c477459aab01c2939368eab2d7e8e0",
    )
    require_normalized_code_sha256(
        "stage3e-read-only-publication",
        "the ordinary publisher must synthesize exactly one verified String constant per ThrowReadOnly and publish its matching typed index before verification",
        stage3b_function(
            consumer_relative,
            "read_trusted_ordinary_function_in_realm",
            "stage3e-read-only-publication",
        ),
        "2dc6e6c5e3a7a1d14c7b79c20dfc7cdb53412b4ee28c9f6fddfc4eec7670ba5d",
    )

    # Stage3D does not introduce a second exception engine. Raw 48 must lower
    # to the engine's existing operand-free Throw instruction, whose verifier,
    # activation driver, unwind path, and public pending slot are locked here.
    if normalized_stack_effect.count("| Self::Throw => (1, 0),") != 1:
        fail(
            "stage3d-throw-verifier",
            "Throw must consume exactly one value and produce no fallthrough value",
        )
    require_normalized_code_sha256(
        "stage3d-throw-verifier",
        "the full typed verifier must keep Throw terminal without a guarded or aliased fallthrough path",
        verify_parts_item,
        "9b3038291ee06873f7b2aaadb61ac884f0d601aefd21e7ea6858d5ee0e746ac1",
    )
    if normalized_verify_parts.count(tail_terminal_dispatch) != 1:
        fail(
            "stage3d-throw-verifier",
            "Throw must remain in the unique terminal verifier arm",
        )
    if (
        normalized_stack_effect.count("| Self::ThrowReadOnly(_)") != 1
        or "Self::ThrowReadOnly(_) => (1, 0)" in normalized_stack_effect
        or normalized_verify_parts.count(
            "Instruction::ThrowReadOnly(_) | Instruction::ThrowRedeclaration(_) | Instruction::ThrowDeleteSuper | Instruction::ThrowIteratorMissingThrow => {}"
        ) != 1
    ):
        fail(
            "stage3e-read-only-verifier",
            "ThrowReadOnly must remain a zero-pop, zero-push terminal verifier node with no fallthrough",
        )
    if (
        normalized_stack_effect.count("Self::Nop | Self::CheckCtor") != 1
        or normalized_verify_parts.count("Instruction::Nop") != 0
        or normalized_verify_parts.count("_ => enqueue_fallthrough(") != 1
    ):
        fail(
            "stage3f-nop-verifier",
            "Instruction::Nop must remain an existing zero-pop, zero-push node that reaches the verifier's unique ordinary fallthrough path",
        )
    if (
        normalized_stack_effect.count("Self::Object => (0, 1)") != 1
        or normalized_verify_parts.count("Instruction::Object") != 0
        or normalized_verify_parts.count("_ => enqueue_fallthrough(") != 1
    ):
        fail(
            "stage3g-object-verifier",
            "Instruction::Object must remain an existing zero-pop, one-push node that reaches the verifier's unique ordinary fallthrough path",
        )
    if (
        normalized_stack_effect.count(
            "Self::SetName(_) | Self::ToObject | Self::IteratorCheckObject => (1, 1)"
        )
        != 1
        or normalized_verify_parts.count("Instruction::ToObject") != 0
        or normalized_verify_parts.count("_ => enqueue_fallthrough(") != 1
    ):
        fail(
            "stage3h-to-object-verifier",
            "Instruction::ToObject must remain an existing one-pop, one-push node that reaches the verifier's unique ordinary fallthrough path",
        )

    execute_hot_item = stage3b_function(
        "src/vm.rs", "execute_hot_instruction", "stage3d-throw-completion"
    )
    throw_vm_arm = unique_braced_item(
        execute_hot_item,
        re.compile(
            r"\bInstruction[ \t\n]*::[ \t\n]*Throw[ \t\n]*=>[ \t\n]*\{"
        ),
        "stage3d-throw-completion",
        "VM Throw dispatch arm",
    )[0]
    if " ".join(throw_vm_arm.split()) != (
        "Instruction::Throw => { return self.pop().map(|value| "
        "Some(Completion::Throw(value))); }"
    ):
        fail(
            "stage3d-throw-completion",
            "VM Throw must pop the original value directly into Completion::Throw",
        )
    throw_read_only_vm_arm = unique_braced_item(
        execute_hot_item,
        re.compile(
            r"\bInstruction[ \t\n]*::[ \t\n]*ThrowReadOnly[ \t\n]*\("
            r"[ \t\n]*index[ \t\n]*\)[ \t\n]*=>[ \t\n]*\{"
        ),
        "stage3e-read-only-completion",
        "VM ThrowReadOnly dispatch arm",
    )[0]
    if " ".join(throw_read_only_vm_arm.split()) != (
        "Instruction::ThrowReadOnly(index) => { "
        "return Err(host.read_only_error(*index)?); }"
    ):
        fail(
            "stage3e-read-only-completion",
            "VM ThrowReadOnly must call the read-only error hook directly without popping or returning",
        )
    nop_vm_arm = unique_braced_item(
        execute_hot_item,
        re.compile(
            r"\bInstruction[ \t\n]*::[ \t\n]*Nop[ \t\n]*=>[ \t\n]*\{"
        ),
        "stage3f-nop-vm",
        "VM Nop dispatch arm",
    )[0]
    if " ".join(nop_vm_arm.split()) != "Instruction::Nop => {}":
        fail(
            "stage3f-nop-vm",
            "VM Nop must remain the existing empty no-effect dispatch arm",
        )
    object_vm_cold_item = stage3b_function(
        "src/vm.rs", "execute_cold_instruction", "stage3g-object-vm"
    )
    require_normalized_code_sha256(
        "stage3h-to-object-vm",
        "execute_cold_instruction must retain its complete reviewed dispatch so ToObject cannot be diverted by a direct, aliased, or helper-mediated pre-match path",
        object_vm_cold_item,
        "1d8fd1a51a5c2e349b2a1055c408a5c877c76661d60373bf22239dae60716cdb",
    )
    object_vm_arm = unique_braced_item(
        object_vm_cold_item,
        re.compile(
            r"\bInstruction[ \t\n]*::[ \t\n]*Object[ \t\n]*=>[ \t\n]*"
            r"match[ \t\n]+host[ \t\n]*\.[ \t\n]*object[ \t\n]*\("
            r"[ \t\n]*\)[ \t\n]*\?[ \t\n]*\{"
        ),
        "stage3g-object-vm",
        "VM Object dispatch arm",
    )[0]
    if " ".join(object_vm_arm.split()) != (
        "Instruction::Object => match host.object()? { "
        "Completion::Return(object) => self.stack.push(object), "
        "Completion::Throw(value) => return Ok(Some(Completion::Throw(value))), }"
    ):
        fail(
            "stage3g-object-vm",
            "VM Object must delegate once to the host, push only its returned fresh Object, and propagate a host throw",
        )
    require_normalized_code_sha256(
        "stage3g-object-realm",
        "the runtime VM host must allocate Object through the executing bytecode's current defining realm",
        stage3b_function(
            "src/runtime/vm_host.rs", "object", "stage3g-object-realm"
        ),
        "90cbeb40094a4266ebba996ce790be75ce46b2ab8959a4996265ddcc656924ce",
    )
    to_object_vm_arm = unique_braced_item(
        object_vm_cold_item,
        re.compile(
            r"\bInstruction[ \t\n]*::[ \t\n]*ToObject[ \t\n]*=>[ \t\n]*\{"
        ),
        "stage3h-to-object-vm",
        "VM ToObject dispatch arm",
    )[0]
    require_normalized_code_sha256(
        "stage3h-to-object-vm",
        "VM ToObject must preserve Object identity, reject nullish values with TypeError, and box only primitives without a coercion hook",
        to_object_vm_arm,
        "7ea7f87dd0d4c20abc0d4148c7e7b2b2b4c4cdbe06be6ac396130138b4123122",
    )
    if re.search(
        r"\b(?:to_primitive|to_property_key|value_of|to_string)[ \t\n]*\(",
        to_object_vm_arm,
    ):
        fail(
            "stage3h-to-object-vm",
            "VM ToObject must not invoke user coercion while preserving Objects or boxing primitives",
        )
    require_normalized_code_sha256(
        "stage3h-to-object-realm",
        "the runtime VM host must allocate every primitive wrapper through the executing bytecode's current defining realm",
        stage3b_function(
            "src/runtime/vm_host.rs", "box_primitive", "stage3h-to-object-realm"
        ),
        "47f1cf4db70f24b86c09ea669b93a0f0a9780ae35a119c5b2f7698f959984ffa",
    )
    require_normalized_code_sha256(
        "stage3d-throw-critical-route",
        "execute_inner must carry raw48 from fetch through the hot dispatcher without a guarded completion alias",
        execute_inner_item,
        "fa323bad632c685546d3efadbe860a77f540b1066559744ea23c333958036358",
    )
    require_normalized_code_sha256(
        "stage3d-throw-critical-route",
        "execute_hot_instruction must enter its unique match before handling Throw and retain the exact dispatch body",
        execute_hot_item,
        "7798feedb7ce69c76c65bf9b7effd4cb80491f6db9ce28c9d1d51b0c51e3db48",
    )
    execute_published_item = stage3b_function(
        "src/vm.rs", "execute_published", "stage3d-throw-critical-route"
    )
    require_normalized_code_sha256(
        "stage3d-throw-critical-route",
        "execute_published must return the activation's Completion directly without post-processing Throw",
        execute_published_item,
        "b2743fde8341d22bb2592d3810e10030ecce6f812befe9be150a80ccd982a0a7",
    )

    runtime_vm_host_relative = "src/runtime/vm_host.rs"
    execute_bytecode_callable_item = stage3b_function(
        runtime_vm_host_relative,
        "execute_bytecode_callable",
        "stage3d-throw-critical-route",
    )
    require_normalized_corridor_sha256(
        "stage3d-throw-critical-route",
        "the module-link bytecode bridge must finish its frame and return execute_published without completion remapping",
        execute_bytecode_callable_item,
        "if is_module_link_entry {",
        "return result.map_err(RuntimeError::Engine);",
        "cc840e26ee0461e8d8951e15459568f47c87e2c5f21e32bc878f38a314c0b57a",
    )
    require_normalized_corridor_sha256(
        "stage3d-throw-critical-route",
        "the normal bytecode bridge must finish its frame and return execute_published without completion remapping",
        execute_bytecode_callable_item,
        "FunctionKind::Normal => {}",
        "result.map_err(RuntimeError::Engine) }",
        "4c6141e7e3aaabf76b78abf7274c2db6864a7feb516ed02c56bac8ebf6af2b60",
    )
    call_internal_item = stage3b_function(
        "src/runtime/native_dispatch.rs",
        "call_internal",
        "stage3d-throw-critical-route",
    )
    require_normalized_code_sha256(
        "stage3d-throw-critical-route",
        "call_internal must preserve the callable completion before, during, and after forwarded-frame cleanup",
        call_internal_item,
        "a94d89cf8db9fb9658f867c9d6af1115c979e571165ff953909d0dbb86e74714",
    )
    require_normalized_corridor_sha256(
        "stage3d-throw-critical-route",
        "call_internal must preserve the callable completion across forwarded-frame cleanup",
        call_internal_item,
        "let result = (|| loop {",
        "frame_error.map_or(result, Err)",
        "3177d4ccf8210565de65aa1534e23c483ae98e25c5b527055b70386d44307735",
    )

    for relative, name, description, expected_hash in (
        (
            runtime_vm_host_relative,
            "call",
            "RuntimeVmHost::call must forward nested callable completions without remapping Throw before caller catch",
            "c1970423cb9f5a75f26e5c309dcc724ee92f74bd48a6e8d24311a3fc94bb6a14",
        ),
        (
            "src/runtime/internal_methods.rs",
            "call_value_internal",
            "call_value_internal must preserve callable and Proxy completions for the current activation",
            "d7564209dc646e4a18641690161eefba207110f7a244377489a96510bb9d66b5",
        ),
        (
            "src/runtime/context.rs",
            "call",
            "Context::call must pass call_internal's completion directly to finish_completion",
            "bf80579858f0ce24fdb44408eb43a1b3a7263bba07ef972026a22a2b1ff0fa89",
        ),
        (
            runtime_vm_host_relative,
            "ensure_backtrace",
            "RuntimeVmHost::ensure_backtrace must delegate explicit Throw values to the runtime backtrace hook",
            "532bdb791b4b0a3e0d4bc1b8bd9658a5c58e321bf864c3786684bbb22006b0d2",
        ),
        (
            runtime_vm_host_relative,
            "iterator_close",
            "RuntimeVmHost::iterator_close must retain getter, call, pending-exception, and result precedence",
            "242106effd28c2885dd94c0cdbb4f85312b650291dc97d7593ff028e83c02aae",
        ),
        (
            runtime_vm_host_relative,
            "read_only_error",
            "RuntimeVmHost::read_only_error must resolve the verified String constant and build the native TypeError in the bytecode realm",
            "b6f792f0992f3b857dc97f0521470557b22593cf86c61f1c079152245ad39f7e",
        ),
        (
            runtime_vm_host_relative,
            "materialize_error",
            "RuntimeVmHost::materialize_error must allocate the native TypeError in the current defining realm before the existing raise path",
            "46fe1d5c192e13c6d1e2f4ae69b6ab04c0b56b0187387c09d2e25a0dab7a0a1f",
        ),
    ):
        require_normalized_code_sha256(
            "stage3d-throw-critical-route",
            description,
            stage3b_function(relative, name, "stage3d-throw-critical-route"),
            expected_hash,
        )
    require_normalized_code_sha256(
        "stage3d-throw-completion",
        "execute must route every Throw completion through the current activation's raise path",
        activation_execute_item,
        "65d316cc1950e983ffc111de71f62e9f9cabb1833acddf93a0980b554df62368",
    )
    require_normalized_code_sha256(
        "stage3d-throw-completion",
        "the suspendable activation driver must share the same Throw raise path",
        activation_run_item,
        "53668ef453a43fc676e96f2c6f01c017f3f336df4bba96674562e77361e0f63e",
    )
    require_normalized_code_sha256(
        "stage3d-throw-completion",
        "raise must attach backtraces before ordered catch and iterator unwinding",
        raise_item,
        "52a25a382122f09433bd29178885bc10c38222aee15f0d5482def7b15b45caab",
    )
    require_ordered_fragments(
        "stage3d-throw-completion",
        "explicit Throw must attach a backtrace, prefer the innermost catch/iterator region order, and preserve the original value across iterator close",
        raise_item,
        (
            "host.ensure_backtrace(&value)?;",
            "let Some(region) = self.regions.pop() else { return Ok(Some(Completion::Throw(value))); };",
            "match region {",
            "VmUnwindRegion::Catch { target, stack_depth, } => {",
            "self.stack.push(value);",
            "VmUnwindRegion::Iterator { record_base, enabled, .. } => {",
            "match host.iterator_close(iterator, true)? {",
            "IteratorCloseOutcome::Closed | IteratorCloseOutcome::Throw(_) => {}",
        ),
    )

    for name, description, expected_hash in (
        (
            "set_pending_exception",
            "the pending-exception writer must retain the original value as an owned runtime root",
            "128c592e4525a60a1bd79dffff836195d8269c54be97a7f17cb98bc5a00a14f8",
        ),
        (
            "take_pending_exception",
            "the pending-exception reader must take and reconstruct the owned original value",
            "8b9051265509db89853144e02a180da15b0851f4d6fbe13bce2b8f3b5743d6de",
        ),
        (
            "has_pending_exception",
            "the pending-exception observer must report the actual pending slot",
            "1812d935455fea3c9027d72b60c252701cc783baa4433a1364050ea4b42e4956",
        ),
    ):
        require_normalized_code_sha256(
            "stage3d-throw-pending",
            description,
            stage3b_function("src/runtime.rs", name, "stage3d-throw-pending"),
            expected_hash,
        )
    for name, description, expected_hash in (
        (
            "has_exception",
            "Context::has_exception must observe the runtime pending slot directly",
            "81a8f856db04b94fd5dd58141a1ed0763d3132fc3ebe3d19dec1645e5367de77",
        ),
        (
            "take_exception",
            "Context::take_exception must return the value taken from the runtime pending slot",
            "691d43e5f989578873bf0ae60896055ed031a9c031c4e4ed74cb908e172bd626",
        ),
    ):
        require_normalized_code_sha256(
            "stage3d-throw-pending",
            description,
            stage3b_function("src/runtime/context.rs", name, "stage3d-throw-pending"),
            expected_hash,
        )

    finish_completion_item = stage3b_function(
        "src/runtime/context.rs", "finish_completion", "stage3d-throw-pending"
    )
    require_normalized_code_sha256(
        "stage3d-throw-pending",
        "the public Context completion bridge must retain the original thrown value in the pending-exception slot",
        finish_completion_item,
        "d2e99ad914f05e1e7d81e0d909d480fae797df41f295a0a7abf806f1ea57ebc9",
    )
    require_ordered_fragments(
        "stage3d-throw-pending",
        "Completion::Throw must become the pending exception before RuntimeError::Exception is returned",
        finish_completion_item,
        (
            "Completion::Return(value) => Ok(value),",
            "Completion::Throw(value) => {",
            "self.runtime.set_pending_exception(value)?;",
            "Err(RuntimeError::Exception)",
        ),
    )

    # Each row pins a compact, semantic ordering contract rather than a brittle
    # whole-function digest. Full-source rewrite canaries exercise the key rows.
    stage3b_ordered_contracts = (
        ("stage3b-validated-construction", "src/runtime.rs", "construct_internal", (
            "constructor: &CallableRef, new_target: &CallableRef,",
            "if !constructor.belongs_to(self) {",
            "if !self.is_constructor(constructor.as_object())? {",
            "if !new_target.belongs_to(self) {",
            "if !self.is_constructor(new_target.as_object())? {",
            "let constructor = ConstructorRef::from_validated_callable(constructor);",
            "let new_target = ConstructorRef::from_validated_callable(new_target);",
            "self.construct_constructor_internal(caller_realm, &constructor, &new_target, arguments)",
        )),
        ("stage3b-raw-construction", "src/runtime.rs", "construct_internal_with_new_target", (
            "constructor: &ConstructorRef, new_target: ConstructNewTarget,",
            "match &new_target { ConstructNewTarget::Validated(new_target) =>",
            "ConstructNewTarget::Raw(new_target) => { self.validate_value_domain(new_target,",
            "if !self.is_constructor(constructor.as_object())? {",
            "if self.is_proxy_object(constructor.as_object())? { return self.construct_proxy(caller_realm, &constructor, new_target, &arguments); }",
            "let callable = self.as_callable(constructor.as_object())?",
            "CallableExecution::Bound",
            "new_target.retarget_bound_identity(&constructor, &target);",
            "constructor = ConstructorRef::from_validated_callable(&target);",
            "CallableExecution::Native",
            "let execution_realm = if target.uses_calling_realm() { caller_realm } else { realm };",
            "return self.construct_native_function( &callable, execution_realm, target, min_readable_args, new_target.value(), &arguments, );",
            "CallableExecution::Bytecode",
            "ConstructorKind::Derived",
            "Value::Undefined, new_target.value(), &arguments,",
            "let raw_new_target = new_target.value();",
            "self.create_from_constructor_value(caller_realm, &raw_new_target)?",
            "this_value.clone(), raw_new_target, &arguments,",
            "CallableExecution::Proxy",
        )),
        ("stage3b-raw-construction", "src/runtime.rs", "construct_value_with_raw_new_target_internal", (
            "let constructor = match self.constructor_from_value(caller_realm, function)?",
            "self.construct_constructor_with_raw_new_target_internal( caller_realm, &constructor, new_target, arguments, )",
        )),
        ("stage3b-raw-construction", "src/runtime.rs", "construct_callable_with_raw_new_target_internal", (
            "constructor_from_value(caller_realm, Value::Object(constructor.as_object().clone()))?",
            "self.construct_constructor_with_raw_new_target_internal( caller_realm, &constructor, new_target, arguments, )",
        )),
        ("stage3b-raw-construction", "src/runtime.rs", "construct_constructor_with_raw_new_target_internal", (
            "self.construct_internal_with_new_target( caller_realm, constructor, ConstructNewTarget::Raw(new_target), arguments, )",
        )),
        ("stage3b-validated-construction", "src/runtime.rs", "construct_constructor_internal", (
            "self.construct_internal_with_new_target( caller_realm, constructor, ConstructNewTarget::Validated(new_target.clone()), arguments, )",
        )),
        ("stage3b-apply-order", "src/runtime/vm_host.rs", "apply", (
            "let callable = self .runtime .callable_from_value(function.clone())",
            "if matches!(argument_array, Value::Undefined | Value::Null) {",
            ".call_internal(self.current_realm, &callable, this_or_new_target, &[])",
            "let arguments = match self.build_argument_list(argument_array)? {",
            "match kind {",
            "ApplyKind::Call => self .runtime .call_internal( self.current_realm, &callable, this_or_new_target, &arguments, )",
            "ApplyKind::Construct => self .runtime .construct_callable_with_raw_new_target_internal( self.current_realm, &callable, this_or_new_target, &arguments, )",
        )),
        ("stage3b-function-realm", "src/runtime/internal_methods.rs", "function_realm_from_value", (
            "self.0.state.borrow().heap.context(caller_realm)?;",
            "let Value::Object(object) = value else { return Ok(NativeConversion::Value(caller_realm)); };",
            "if !object.belongs_to(self) {",
            "self.function_realm_object_impl(Some(caller_realm), object.clone(), true)",
        )),
        ("stage3b-function-realm", "src/runtime/internal_methods.rs", "function_realm_object_impl", (
            "ObjectPayload::NativeFunction { data, .. } if data.realm.is_some() =>",
            "if allow_non_function && data.target.uses_calling_realm() {",
            "ObjectPayload::BytecodeFunction",
            "ObjectPayload::BoundFunction { target, .. } =>",
            "ObjectPayload::Proxy(data) =>",
            "if is_revoked {",
            "if allow_non_function {",
        )),
        ("stage3b-constructor-prototype", "src/runtime.rs", "constructor_prototype_source", (
            "if matches!(new_target, Value::Undefined) {",
            "let prototype = match self.get_value_property_in_realm( caller_realm, new_target.clone(), &prototype_key, )? {",
            "if let Value::Object(prototype) = prototype {",
            "self.function_realm_from_value(caller_realm, new_target) .map(|result| match result {",
            "NativeConversion::Value(realm) => { NativeConversion::Value(ConstructorPrototypeSource::Realm(realm)) }",
        )),
        ("stage3b-constructor-prototype", "src/runtime.rs", "prototype_from_constructor_value", (
            "match self.constructor_prototype_source(caller_realm, new_target)? {",
            "ConstructorPrototypeSource::Explicit(prototype)",
            "ConstructorPrototypeSource::Realm(realm)",
            "fallback(realm).map(NativeConversion::Value)",
            "NativeConversion::Throw(value) => Ok(NativeConversion::Throw(value))",
        )),
        ("stage3b-proxy-call-order", "src/runtime/internal_methods.rs", "call_proxy", (
            "let data = self .proxy_snapshot_if_any(&current)?",
            "if data.is_revoked {",
            "let rooted = self.root_proxy_snapshot(&current, data)?;",
            "let method = match self.internal_get(",
            "if !rooted.data.is_callable {",
            "if matches!(method, Value::Undefined | Value::Null) {",
            "if self.is_proxy_object(&rooted.target)? {",
            "current = rooted.target.clone();",
            "depth = depth.saturating_add(1);",
            "continue;",
            "return self.call_value_internal( realm, Value::Object(rooted.target.clone()), this_value, arguments, );",
            "let argument_array = self.new_array_from_values(realm, arguments.to_vec())?;",
            "let method = match self.direct_call_target_from_value(method) {",
            "&[ Value::Object(rooted.target.clone()), this_value, Value::Object(argument_array), ],",
        )),
        ("stage3b-proxy-construct-order", "src/runtime/internal_methods.rs", "construct_proxy", (
            "proxy: &ConstructorRef, new_target: ConstructNewTarget,",
            "let data = self.proxy_snapshot_if_any(current.as_object())?",
            "if data.is_revoked {",
            "let rooted = self.root_proxy_snapshot(current.as_object(), data)?;",
            "let method = match self.internal_get(",
            "let target = match self.constructor_from_value(realm, Value::Object(rooted.target.clone()))?",
            "if matches!(method, Value::Undefined | Value::Null) {",
            "if self.is_proxy_object(target.as_object())? {",
            "current = target;",
            "depth = depth.saturating_add(1);",
            "continue;",
            ".construct_internal_with_new_target(realm, &target, new_target, arguments);",
            "let argument_array = self.new_array_from_values(realm, arguments.to_vec())?;",
            "let method = match self.direct_call_target_from_value(method) {",
            "let result = self.call_proxy_trap(",
            "&[ Value::Object(rooted.target.clone()), Value::Object(argument_array), new_target.value(), ],",
            "return match result {",
        )),
        ("stage3b-public-construction", "src/runtime/context.rs", "construct_with_new_target", (
            "constructor: &CallableRef",
            "new_target: &CallableRef",
            ".construct_internal(self.realm, constructor, new_target, arguments)",
        )),
        ("stage3b-public-construction", "src/runtime/intrinsics/reflect.rs", "call_reflect_construct", (
            "let explicit_new_target = if arguments.actual_arg_count > 2 {",
            "self.constructor_from_value(realm, value)?",
            "let forwarded = match self.build_array_like_argument_list(",
            "let target = match self.constructor_from_value(realm, target_value)?",
            "let new_target = explicit_new_target.unwrap_or_else(|| target.clone());",
            "self.construct_constructor_internal(realm, &target, &new_target, &forwarded)",
        )),
        ("stage3b-species-constructor", "src/runtime/intrinsics/array_buffer/typed_array/species.rs", "typed_array_create_from_constructor_arguments", (
            "if !self.is_constructor(&object)? {",
            "let constructor = match self.constructor_from_value(realm, Value::Object(object))?",
            "let target = match self.construct_constructor_internal( realm, &constructor, &constructor, arguments, )? {",
        )),
        ("stage3b-species-constructor", "src/runtime/intrinsics/array.rs", "array_from_result", (
            "let constructor = match self.constructor_from_value(realm, constructor)?",
            "let arguments = length.into_iter().collect::<Vec<_>>();",
            "return self.construct_constructor_internal( realm, &constructor, &constructor, &arguments, );",
        )),
        ("stage3b-species-constructor", "src/runtime/intrinsics/array.rs", "call_array_of", (
            "let constructor = match self.constructor_from_value(realm, this_value)?",
            "match self.construct_constructor_internal( realm, &constructor, &constructor, &[Self::array_length_value(length)], )? {",
        )),
        ("stage3b-species-constructor", "src/runtime/intrinsics/array.rs", "array_species_create", (
            "let constructor = match self.constructor_from_value(realm, constructor)?",
            "self.construct_constructor_internal( realm, &constructor, &constructor, &[Value::number(length as f64)], )",
        )),
    )
    for diagnostic, relative, function_name, fragments in stage3b_ordered_contracts:
        require_ordered_fragments(
            diagnostic,
            f"{relative}::{function_name} must retain its reviewed Stage3B branch order",
            stage3b_function(relative, function_name, diagnostic),
            fragments,
        )

    construct_dispatch = stage3b_function(
        "src/runtime.rs", "construct_internal_with_new_target", "stage3b-raw-construction"
    )
    normalized_construct_dispatch = " ".join(construct_dispatch.split())
    if (
        normalized_construct_dispatch.count("new_target.value()") != 3
        or normalized_construct_dispatch.count("let raw_new_target") != 1
    ):
        fail("stage3b-raw-construction", "native, derived, and base paths must preserve one raw newTarget flow")

    apply_host = stage3b_function("src/runtime/vm_host.rs", "apply", "stage3b-apply-order")
    if " ".join(apply_host.split()).count("build_argument_list(") != 1:
        fail("stage3b-apply-order", "Apply must build a nonnull argument list exactly once")

    realm_object_impl = stage3b_function(
        "src/runtime/internal_methods.rs", "function_realm_object_impl", "stage3b-function-realm"
    )
    if " ".join(realm_object_impl.split()).count(
        "object = ObjectRef::from_borrowed_handle(self.clone(), target)?;"
    ) != 2:
        fail("stage3b-function-realm", "bound and Proxy realm traversal must each advance to their target")

    prototype_helper = stage3b_function(
        "src/runtime.rs", "prototype_from_constructor_value", "stage3b-constructor-prototype"
    )
    if re.search(r"\b(?:CallableRef|callable_from_value|as_callable)\b", prototype_helper):
        fail("stage3b-constructor-prototype", "prototype fallback must consume raw newTarget")

    native_borrowed_prototype_consumers = (
        ("src/runtime.rs", "create_from_constructor_value"),
        ("src/runtime/intrinsics/array.rs", "create_array_from_constructor"),
    )
    native_owned_prototype_consumers = (
        ("src/runtime.rs", "call_function_constructor"),
        ("src/runtime.rs", "call_primitive_constructor"),
        ("src/runtime/intrinsics/array_buffer.rs", "array_buffer_prototype_from_new_target"),
        ("src/runtime/intrinsics/error.rs", "call_error_constructor"),
        ("src/runtime/intrinsics/iterator.rs", "iterator_prototype_from_new_target"),
        ("src/runtime/intrinsics/map.rs", "map_prototype_from_new_target"),
        ("src/runtime/intrinsics/promise.rs", "promise_prototype_from_new_target"),
        ("src/runtime/intrinsics/set.rs", "set_prototype_from_new_target"),
        ("src/runtime/intrinsics/shared_array_buffer.rs", "shared_array_buffer_prototype_from_new_target"),
        ("src/runtime/intrinsics/weak_collection.rs", "weak_collection_prototype_from_new_target"),
        ("src/runtime/intrinsics/weak_ref.rs", "weak_intrinsic_prototype_from_new_target"),
        ("src/runtime/intrinsics/array_buffer/data_view.rs", "data_view_prototype_from_new_target"),
        ("src/runtime/intrinsics/array_buffer/typed_array.rs", "typed_array_prototype_from_new_target"),
        ("src/runtime/intrinsics/date/constructor.rs", "date_prototype_from_new_target"),
        ("src/runtime/intrinsics/regexp/constructor.rs", "allocate_regexp_from_new_target"),
    )
    native_prototype_consumers = (
        *((relative, function_name, "new_target") for relative, function_name in native_borrowed_prototype_consumers),
        *((relative, function_name, "&new_target") for relative, function_name in native_owned_prototype_consumers),
    )
    for relative, function_name, new_target_argument in native_prototype_consumers:
        item = stage3b_function(relative, function_name, "stage3b-native-prototype-family")
        if (
            " ".join(item.split()).count("prototype_from_constructor_value(") != 1
            or " ".join(item.split()).count(f", {new_target_argument},") != 1
            or re.search(r"\b(?:CallableRef|callable_from_value)\b", item)
        ):
            fail("stage3b-native-prototype-family", f"{relative}::{function_name} bypasses the raw helper payload")

    constructor_only_items = (
        ("stage3b-proxy-construct-order", "src/runtime/internal_methods.rs", "construct_proxy"),
        ("stage3b-species-constructor", "src/runtime/intrinsics/array_buffer/typed_array/species.rs", "typed_array_create_from_constructor_arguments"),
        ("stage3b-species-constructor", "src/runtime/intrinsics/array.rs", "array_from_result"),
        ("stage3b-species-constructor", "src/runtime/intrinsics/array.rs", "call_array_of"),
        ("stage3b-species-constructor", "src/runtime/intrinsics/array.rs", "array_species_create"),
    )
    for diagnostic, relative, function_name in constructor_only_items:
        if re.search(
            r"\b(?:CallableRef|callable_from_value|as_callable)\b",
            stage3b_function(relative, function_name, diagnostic),
        ):
            fail(diagnostic, f"{relative}::{function_name} must preserve constructor-only capability")

    context_construct = stage3b_function(
        "src/runtime/context.rs", "construct_with_new_target", "stage3b-public-construction"
    )
    if "raw_new_target" in " ".join(context_construct.split()):
        fail("stage3b-public-construction", "Context must not expose the raw VM construction seam")

    species_functions = (
        ("src/runtime/intrinsics/array_buffer.rs", "array_buffer_species_constructor", True),
        ("src/runtime/intrinsics/shared_array_buffer.rs", "shared_array_buffer_species_constructor", True),
        ("src/runtime/intrinsics/promise.rs", "promise_species_constructor", True),
        ("src/runtime/intrinsics/regexp/constructor.rs", "regexp_species_constructor", False),
    )
    for relative, function_name, wraps_option in species_functions:
        item = stage3b_function(relative, function_name, "stage3b-species-constructor")
        normalized_item = " ".join(item.split())
        if (
            "ConstructorRef" not in normalized_item
            or normalized_item.count("constructor_from_value(") != 1
            or (
                wraps_option
                and normalized_item.count(
                    "NativeConversion::Value(constructor) => NativeConversion::Value(Some(constructor))"
                ) != 1
            )
            or re.search(r"\b(?:CallableRef|callable_from_value|as_callable)\b", item)
        ):
            fail("stage3b-species-constructor", f"{relative}::{function_name} narrows species capability")

    stage3b_runtime_test_contracts = (
        ("trusted_quickjs_ordinary_apply_admits_only_canonical_typed_kinds", (
            "for (magic, expected_kind) in [(0, ApplyKind::Call), (1, ApplyKind::Construct)] {",
            "for magic in [2_u16, u16::MAX] {",
            "assert_eq!(runtime.heap_counts(), baseline);",
        )),
        ("trusted_quickjs_ordinary_apply_preserves_object_list_call_and_construct_semantics", (
            "&[target, receiver, list]",
            "&[constructor, new_target.clone(), list]",
            "context.get_property(&instance, &sum).unwrap()",
        )),
        ("trusted_quickjs_ordinary_apply_nullish_lists_use_raw_receiver_for_both_kinds", (
            "assert!(!runtime.is_constructor(target_object).unwrap());",
            "for magic in [0_u8, 1] {",
            "for argument_array in [Value::Null, Value::Undefined] {",
        )),
        ("trusted_quickjs_ordinary_apply_preserves_error_order_realm_and_pending_identity", (
            "native_error_prototypes[NativeErrorKind::Type.index()]",
            "caller.take_exception().unwrap().unwrap()",
            "&[target, receiver.clone(), Value::Null]",
        )),
        ("trusted_quickjs_ordinary_apply_raw_prototype_get_preserves_order_and_receiver", (
            "Value::Int(17), empty.clone()",
            "context.take_exception().unwrap().unwrap()",
            "Some(context.object_prototype().unwrap())",
        )),
        ("trusted_quickjs_ordinary_apply_construct_preserves_raw_new_target_across_dispatch", (
            "NativeFunctionId::ConstructorProbe",
            "set_constructor_bit(native.as_object(), true)",
            "Value::Int(17), empty.clone()",
        )),
        ("construct_only_proxy_and_new_target_do_not_require_call_capability", (
            "runtime.as_callable(&proxy).unwrap().is_none()",
            "VmHost::construct(",
            "runtime.set_constructor_bit(&new_target, true)",
        )),
        ("trusted_quickjs_ordinary_apply_raw_calling_realm_functions_fall_back_to_the_caller", (
            "execute_pending_job_with_context()",
            "Some(caller.realm)",
            "runtime.set_constructor_bit(&revoke_object, false)",
        )),
        ("trusted_quickjs_ordinary_apply_raw_native_constructors_use_class_fallbacks", (
            "NativeFunctionId::ArrayConstructor",
            "for (label, constructor_source, arguments_source) in cases {",
            "Some(expected_prototype)",
        )),
        ("trusted_quickjs_ordinary_apply_verifies_stack_and_reindexed_branch_targets", (
            "const UNDERFLOW: &str",
            "for (label, code, max_stack, expected) in [",
            "Instruction::Goto(9)",
            "Instruction::Apply(ApplyKind::Call)",
        )),
    )
    runtime_tests_code = rust_code_only(read_source("src/runtime/tests.rs"))
    missing_stage3b_tests = []
    for name, _ in stage3b_runtime_test_contracts:
        declarations = list(re.finditer(
            rf"\bfn[ \t\n]+{name}[ \t\n]*\(", runtime_tests_code
        ))
        if len(declarations) != 1:
            missing_stage3b_tests.append(name)
            continue
        declaration = declarations[0]
        previous_item_end = runtime_tests_code.rfind("}", 0, declaration.start()) + 1
        attributes = runtime_tests_code[previous_item_end:declaration.start()]
        if " ".join(attributes.split()) != "#[test]":
            missing_stage3b_tests.append(name)
    drifted_stage3b_tests = []
    for name, anchors in stage3b_runtime_test_contracts:
        item = stage3b_function("src/runtime/tests.rs", name, "stage3b-runtime-evidence")
        normalized_item = " ".join(item.split())
        if any(anchor not in normalized_item for anchor in anchors):
            drifted_stage3b_tests.append(name)
    if (
        missing_stage3b_tests
        or drifted_stage3b_tests
        or runtime_tests_code.count("for magic in [2_u16, u16::MAX]") != 1
        or function_translate_code.count("for magic in [2, u16::MAX]") != 1
    ):
        fail(
            "stage3b-runtime-evidence",
            "Stage3B must retain its canonical/noncanonical, ordering, raw propagation, realm, native-family, Proxy, stack, and branch regression matrix; "
            f"missing {missing_stage3b_tests}, drifted {drifted_stage3b_tests}",
        )

    stage3c_test_module_contracts = (
        ("src/runtime.rs", ";"),
        ("src/vm.rs", "{"),
        ("src/bytecode.rs", "{"),
        ("src/runtime/binary_object/function_translate/mod.rs", "{"),
        ("src/runtime/binary_object/ordinary_leaf.rs", "{"),
        ("src/runtime/binary_object_publish.rs", "{"),
        ("src/runtime/binary_object/function_translate/capability.rs", "{"),
    )
    test_module_pattern = re.compile(
        r"(?m)(?P<attributes>(?:^[ \t]*#[ \t]*\[[^]\n]*\][ \t]*\n)*)"
        r"^[ \t]*mod[ \t]+tests[ \t]*(?P<form>;|\{)"
    )
    drifted_stage3c_test_modules = []
    for relative, expected_form in stage3c_test_module_contracts:
        code = stage3b_code(relative)
        declarations = [
            declaration
            for declaration in test_module_pattern.finditer(code)
            if code[:declaration.start()].count("{")
            == code[:declaration.start()].count("}")
        ]
        if (
            len(declarations) != 1
            or " ".join(declarations[0].group("attributes").split()) != "#[cfg(test)]"
            or declarations[0].group("form") != expected_form
        ):
            drifted_stage3c_test_modules.append(relative)

    stage3c_required_test_sources = {
        "src/runtime/tests.rs",
        "src/vm.rs",
        "src/bytecode.rs",
        "src/runtime/binary_object/function_translate/mod.rs",
        "src/runtime/binary_object/ordinary_leaf.rs",
        "src/runtime/binary_object_publish.rs",
        "src/runtime/binary_object/function_translate/capability.rs",
    }
    for relative in stage3c_required_test_sources:
        if re.search(
            r"(?m)^[ \t]*#![ \t]*\[[ \t]*(?:cfg|cfg_attr)\b",
            stage3b_code(relative),
        ):
            drifted_stage3c_test_modules.append(relative)
    if drifted_stage3c_test_modules:
        fail(
            "stage3c-runtime-evidence",
            "every required Stage3C test module must retain exactly #[cfg(test)] and no outer or inner cfg/cfg_attr exclusion; "
            f"drifted {sorted(set(drifted_stage3c_test_modules))}",
        )

    stage3c_test_contracts = (
        (
            "src/runtime/binary_object/function_translate/capability.rs",
            "registry_locks_the_current_physical_cohorts",
            (
                "(111, 1, 103, 29)",
                "assert_eq!(ordinary_only + shared, 132);",
                "assert_eq!(scalar_only + ordinary_only + shared, 133);",
            ),
        ),
        (
            "src/runtime/binary_object/function_translate/capability.rs",
            "ordinary_invocation_addition_is_the_exact_reviewed_six_row_set",
            (
                "(35, OpcodeFormat::NPop, Recipe::TailCall)",
                "(37, OpcodeFormat::NPop, Recipe::TailCallMethod)",
                "CapabilityPolicy::OrdinaryOnly(expected_recipe)",
            ),
        ),
        (
            "src/runtime/binary_object/function_translate/capability.rs",
            "blocked_frontier_has_stable_typed_category_counts",
            (
                "let mut counts = [0_usize; 15];",
                "assert!(counts.into_iter().all(|count| count != 0));",
                "assert_eq!(counts.into_iter().sum::<usize>(), 111);",
            ),
        ),
        (
            "src/runtime/binary_object/function_translate/mod.rs",
            "tail_invocation_lowering_preserves_the_npop_operand_and_kind",
            (
                "NativeOperands::NPop(u16::MAX)",
                "(false, FunctionOp::TailCall(u16::MAX))",
                "(true, FunctionOp::TailCallMethod(u16::MAX))",
            ),
        ),
        (
            "src/runtime/binary_object/ordinary_leaf.rs",
            "tail_invocation_operands_reach_the_ordinary_dto_unchanged",
            (
                "FunctionOp::TailCall(u16::MAX)",
                "OrdinaryLeafOp::TailCall(u16::MAX)",
                "OrdinaryLeafOp::TailCallMethod(u16::MAX)",
            ),
        ),
        (
            "src/runtime/binary_object_publish.rs",
            "ordinary_tail_invocation_publishes_one_for_one_with_the_unchanged_operand",
            (
                "OrdinaryLeafOp::TailCall(u16::MAX)",
                "Instruction::TailCall(u16::MAX)",
                "Instruction::TailCallMethod(u16::MAX)",
                "assert_eq!(next_synthetic_index, 0);",
            ),
        ),
        (
            "src/bytecode.rs",
            "verifier_models_tail_invocations_as_terminal_zero_result_operations",
            (
                "Instruction::TailCall(2)",
                "Instruction::TailCallMethod(2)",
                "for instruction in [Instruction::TailCall(0), Instruction::TailCallMethod(0)]",
                "Instruction::TailCall(u16::MAX)",
                "Instruction::TailCallMethod(u16::MAX)",
            ),
        ),
        (
            "src/vm.rs",
            "tail_invocations_complete_the_frame_with_exact_call_operands",
            (
                "Completion::Return(Value::Int(42))",
                "Value::Undefined, vec![Value::Int(11), Value::Int(12)]",
                "Value::Int(21), Value::Int(20), vec![Value::Int(22), Value::Int(23)]",
            ),
        ),
        (
            "src/vm.rs",
            "tail_invocation_throws_use_the_activation_backtrace_and_catch_path",
            (
                "Completion::Throw(Value::Int(77))",
                "assert_eq!(host.backtrace_values, [Value::Int(77)]);",
                "assert_eq!(host.captured_local_reuse_preparations, 1);",
                "Completion::Throw(Value::Int(88))",
            ),
        ),
        (
            "src/runtime/tests.rs",
            "trusted_quickjs_ordinary_tail_invocations_use_exact_bc5_wires_and_semantics",
            (
                "assert_eq!(QUICKJS_ORDINARY_TAIL_CALL_BC5.len(), 57);",
                "0x8ff9_d2c1_0c7e_2228",
                "assert_eq!(QUICKJS_ORDINARY_TAIL_CALL_METHOD_BC5.len(), 62);",
                "0xe87d_54c0_a2a1_40ca",
                "Instruction::TailCall(2)",
                "Instruction::TailCallMethod(2)",
            ),
        ),
        (
            "src/runtime/tests.rs",
            "trusted_quickjs_ordinary_tail_invocation_failures_are_recoverable",
            (
                "vec![Value::Int(0), Value::Int(1), Value::Int(2)]",
                "vec![Value::Null, Value::Int(0), Value::Int(1), Value::Int(2)]",
                "Err(RuntimeError::Exception)",
                "Value::Int(42)",
            ),
        ),
        (
            "src/runtime/tests.rs",
            "trusted_quickjs_ordinary_tail_verification_rolls_back_heap_and_atoms",
            (
                "quickjs_ordinary_three_argument_with_code(&[0x23, 0xff, 0xff], 0)",
                "quickjs_ordinary_four_argument_with_code(&[0x25, 0xff, 0xff], 0)",
                "assert_eq!(runtime.heap_counts(), baseline",
                "assert_eq!(runtime.test_atom_count(), baseline_atoms",
            ),
        ),
        (
            "src/runtime/tests.rs",
            "published_tail_call_uses_backtrace_and_current_activation_catch_semantics",
            (
                "Instruction::TailCall(0)",
                "let stack = own_stack_string(&runtime, &error).to_utf8_lossy();",
                "Instruction::Catch(4)",
                "Value::Int(17)",
            ),
        ),
    )
    stage3c_test_body_hashes = {
        ("src/runtime/binary_object/function_translate/capability.rs", "registry_locks_the_current_physical_cohorts"): "3ff8521f17f3acd991542c7de3e769507ba061eb19d605a413e4febe65f77cc9",
        ("src/runtime/binary_object/function_translate/capability.rs", "ordinary_invocation_addition_is_the_exact_reviewed_six_row_set"): "0bd25bc945bde404ea911c720491e06a0d58c7257683347372b6c74843d3afc2",
        ("src/runtime/binary_object/function_translate/capability.rs", "blocked_frontier_has_stable_typed_category_counts"): "601aa4392300e4a3f965cef80787945d4aea706fdc729325318a016e17bb41c8",
        ("src/runtime/binary_object/function_translate/mod.rs", "tail_invocation_lowering_preserves_the_npop_operand_and_kind"): "f9a36b2d07545f80276edce3e1e7f3e1baaa56fcdfd82996a857be1958acccd2",
        ("src/runtime/binary_object/ordinary_leaf.rs", "tail_invocation_operands_reach_the_ordinary_dto_unchanged"): "fd78ceeb26a50bc988ebb4fdece4f4417b89abd31bbfe0edae6480899e8c1ecc",
        ("src/runtime/binary_object_publish.rs", "ordinary_tail_invocation_publishes_one_for_one_with_the_unchanged_operand"): "7277c89d2d824c8bfe59cecc5ed5fdbf4e68f0368b719794d3072df7782bc94e",
        ("src/bytecode.rs", "verifier_models_tail_invocations_as_terminal_zero_result_operations"): "c9e3ab6c07492a5e74e91c689b8f70ef324b74ffce9321e92460fc356fe2af07",
        ("src/vm.rs", "tail_invocations_complete_the_frame_with_exact_call_operands"): "08a72d3c502b9610b9efa10dcdca4bbcfec90d7ffcd76ded373e97f8e8982dac",
        ("src/vm.rs", "tail_invocation_throws_use_the_activation_backtrace_and_catch_path"): "b04b8f4116f8988c79c8ed41f6825f996fd15225fd871a6d44708d7e9c344fae",
        ("src/runtime/tests.rs", "trusted_quickjs_ordinary_tail_invocations_use_exact_bc5_wires_and_semantics"): "4b972572d236f13acf54b98365cd4c8ad52bd86e5a9dc97cbb67b727a8540562",
        ("src/runtime/tests.rs", "trusted_quickjs_ordinary_tail_invocation_failures_are_recoverable"): "0640226781baa7b22b4a68b2c1d0929ee1c60e09c347c1ebd9f23ed21bfd6235",
        ("src/runtime/tests.rs", "trusted_quickjs_ordinary_tail_verification_rolls_back_heap_and_atoms"): "e089f00738beddf2de2926d27f59850eb34b4a806184177deb9922228a0cc767",
        ("src/runtime/tests.rs", "published_tail_call_uses_backtrace_and_current_activation_catch_semantics"): "46a75e60a1f007a7b626bc0579aeb7435911a47199bf0accc0a2c5dfa4e7cd06",
    }
    missing_stage3c_tests = []
    drifted_stage3c_tests = []
    for relative, name, anchors in stage3c_test_contracts:
        code = stage3b_code(relative)
        declarations = list(re.finditer(
            rf"(?P<attributes>(?:#[ \t\n]*\[[^]]*\][ \t\n]*)*)"
            rf"\bfn[ \t\n]+{re.escape(name)}[ \t\n]*\(",
            code,
        ))
        if (
            len(declarations) != 1
            or " ".join(declarations[0].group("attributes").split()) != "#[test]"
        ):
            missing_stage3c_tests.append(f"{relative}::{name}")
            continue
        item = stage3b_function(relative, name, "stage3c-runtime-evidence")
        normalized_item = " ".join(item.split())
        if (
            any(anchor not in normalized_item for anchor in anchors)
            or normalized_code_sha256(item)
            != stage3c_test_body_hashes.get((relative, name))
        ):
            drifted_stage3c_tests.append(f"{relative}::{name}")
    if missing_stage3c_tests or drifted_stage3c_tests:
        fail(
            "stage3c-runtime-evidence",
            "Stage3C tests must retain exact #[test] attributes and the typed-chain, BC5, verifier, operand-order, completion, catch, backtrace, unwind, recovery, and rollback evidence; "
            f"missing {missing_stage3c_tests}, drifted {drifted_stage3c_tests}",
        )

    stage3d_runtime_tests_code = stage3b_code("src/runtime/tests.rs")
    stage3d_throw_wire_matches = list(re.finditer(
        r"\bconst[ \t\n]+QUICKJS_ORDINARY_THROW_BC5[ \t\n]*:"
        r"[ \t\n]*&[ \t\n]*\[[ \t\n]*u8[ \t\n]*\][ \t\n]*="
        r"[ \t\n]*&[ \t\n]*\[(?P<body>[^]]*)\][ \t\n]*;",
        stage3d_runtime_tests_code,
    ))
    stage3d_throw_wire = b""
    if len(stage3d_throw_wire_matches) != 1:
        fail(
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
        fnv = 0xCBF29CE484222325
        for byte in stage3d_throw_wire:
            fnv ^= byte
            fnv = (fnv * 0x100000001B3) & 0xFFFFFFFFFFFFFFFF
        if (
            wire_residue
            or len(stage3d_throw_wire) != 45
            or fnv != 0x73CF217E06C5FEE2
            or hashlib.sha256(stage3d_throw_wire).hexdigest()
            != "b7998b9678635e7e0a4eb2e465b683d168395adc7f156f733c25521907e3c8a8"
            or stage3d_throw_wire[-2:] != bytes((0xCF, 0x30))
        ):
            fail(
                "stage3d-runtime-evidence",
                "the Rust raw48 fixture must remain the exact 45-byte cf30 wire with its frozen FNV-1a-64 and SHA-256",
            )

    def stage3e_fnv1a64(payload: bytes) -> int:
        value = 0xCBF29CE484222325
        for byte in payload:
            value ^= byte
            value = (value * 0x100000001B3) & 0xFFFFFFFFFFFFFFFF
        return value

    def stage3e_byte_array(
        source: str,
        name: str,
        diagnostic: str = "stage3e-runtime-evidence",
    ) -> bytes:
        matches = list(re.finditer(
            rf"\bconst[ \t\n]+{re.escape(name)}[ \t\n]*:"
            r"[ \t\n]*&[ \t\n]*\[[ \t\n]*u8[ \t\n]*\][ \t\n]*="
            r"[ \t\n]*&[ \t\n]*\[(?P<body>[^]]*)\][ \t\n]*;",
            source,
        ))
        if len(matches) != 1:
            fail(
                diagnostic,
                f"the Rust runtime evidence must retain exactly one literal {name} wire",
            )
            return b""
        body = matches[0].group("body")
        tokens = re.findall(r"0x[0-9A-Fa-f]+|[0-9]+", body)
        residue = re.sub(r"0x[0-9A-Fa-f]+|[0-9]+|[\s,]", "", body)
        if residue:
            fail(
                diagnostic,
                f"{name} must contain only literal byte tokens",
            )
            return b""
        try:
            return bytes(int(token, 0) for token in tokens)
        except ValueError:
            fail(diagnostic, f"{name} contains an invalid byte")
            return b""

    def stage3e_concat_hex(source: str, name: str) -> bytes:
        matches = list(re.finditer(
            rf"\bconst[ \t\n]+{re.escape(name)}[ \t\n]*:[^=;]+="
            r"[ \t\n]*concat![ \t\n]*\((?P<body>.*?)\)[ \t\n]*;",
            source,
            re.DOTALL,
        ))
        if len(matches) != 1:
            fail(
                "stage3e-runtime-evidence",
                f"the archive evidence must retain exactly one {name} concat wire",
            )
            return b""
        body = matches[0].group("body")
        chunks = re.findall(r'"([0-9A-Fa-f]*)"', body)
        residue = re.sub(r'"[0-9A-Fa-f]*"|[\s,]', "", body)
        joined = "".join(chunks)
        if residue or len(joined) % 2:
            fail(
                "stage3e-runtime-evidence",
                f"{name} must contain only an even-length literal hex concat",
            )
            return b""
        try:
            return bytes.fromhex(joined)
        except ValueError:
            fail("stage3e-runtime-evidence", f"{name} contains invalid hex")
            return b""

    stage3e_runtime_source = read_source("src/runtime/tests.rs")
    stage3e_ordinary_source = read_source(
        "src/runtime/binary_object/ordinary_leaf.rs"
    )
    stage3e_manual_wire = stage3e_byte_array(
        stage3e_runtime_source, "QUICKJS_ORDINARY_READ_ONLY_BC5"
    )
    stage3e_natural_wire = stage3e_byte_array(
        stage3e_runtime_source, "QUICKJS_NATURAL_READ_ONLY_BC5"
    )
    stage3e_manual_hex_wire = stage3e_concat_hex(
        stage3e_ordinary_source, "READ_ONLY_LEAF_HEX"
    )
    stage3e_natural_hex_wire = stage3e_concat_hex(
        stage3e_ordinary_source, "NATURAL_READ_ONLY_LEAF_HEX"
    )
    stage3e_wire_contracts = (
        (
            "manual raw49",
            stage3e_manual_wire,
            stage3e_manual_hex_wire,
            47,
            0xB4C1126C283093AF,
            "d05cabd4c18598b024f66eab8fd723c412fc5a469325b26fca5042507dea3ee8",
            bytes.fromhex("31f300000000"),
        ),
        (
            "natural raw49 origin",
            stage3e_natural_wire,
            stage3e_natural_hex_wire,
            58,
            0x026914EDA60A481F,
            "a07b3f39a5e3929af4899a07686e91324e4ee9c54b729f518813eaa4a1875199",
            bytes.fromhex("5e0000b3c7b41131f300000000"),
        ),
    )
    for label, runtime_wire, archive_wire, size, fnv, sha256, child_code in stage3e_wire_contracts:
        if (
            runtime_wire != archive_wire
            or len(runtime_wire) != size
            or stage3e_fnv1a64(runtime_wire) != fnv
            or hashlib.sha256(runtime_wire).hexdigest() != sha256
            or not runtime_wire.endswith(child_code)
        ):
            fail(
                "stage3e-runtime-evidence",
                f"the Rust {label} fixture must retain its exact byte identity, FNV-1a-64, SHA-256, and child code",
            )

    stage3f_nop_wire = stage3e_byte_array(
        stage3e_runtime_source, "QUICKJS_ORDINARY_NOP_BC5"
    )
    if (
        len(stage3f_nop_wire) != 41
        or stage3e_fnv1a64(stage3f_nop_wire) != 0x1C522736E3CBEF92
        or hashlib.sha256(stage3f_nop_wire).hexdigest()
        != "26c2e58ec14861dc797a7c3a3701f258ba392b649a15554256b61d7634fccdd0"
        or stage3f_nop_wire[1] != 0
        or stage3f_nop_wire[25:29] != bytes.fromhex("0c430201")
        or stage3f_nop_wire[30:37] != bytes(7)
        or stage3f_nop_wire[37:]
        != bytes.fromhex("0200b129")
    ):
        fail(
            "stage3f-runtime-evidence",
            "the Rust raw177 fixture must retain the exact 41-byte property-free zero-stack raw177/raw41 wire with no atoms or constants",
        )

    stage3g_object_wire = stage3e_byte_array(
        stage3e_runtime_source,
        "QUICKJS_ORDINARY_OBJECT_BC5",
        "stage3g-runtime-evidence",
    )
    if (
        len(stage3g_object_wire) != 41
        or stage3e_fnv1a64(stage3g_object_wire) != 0x3C41AF3FEF8B3A1E
        or hashlib.sha256(stage3g_object_wire).hexdigest()
        != "a58ccbed5658ba6a9de99e909d5ba0b4af59ad47fccf0f5cccdff072d6494db9"
        or not stage3g_object_wire.endswith(bytes.fromhex("0b28"))
    ):
        fail(
            "stage3g-runtime-evidence",
            "the Rust raw11 fixture must retain the exact compiler-natural 41-byte Object/Return wire with FNV-1a-64 3c41af3fef8b3a1e, SHA-256 a58ccbed5658ba6a9de99e909d5ba0b4af59ad47fccf0f5cccdff072d6494db9, and child code 0b28",
        )

    stage3h_natural_to_object_wire = stage3e_byte_array(
        stage3e_runtime_source,
        "QUICKJS_NATURAL_TO_OBJECT_BC5",
        "stage3h-runtime-evidence",
    )
    stage3h_manual_to_object_wire = stage3e_byte_array(
        stage3e_runtime_source,
        "QUICKJS_ORDINARY_TO_OBJECT_BC5",
        "stage3h-runtime-evidence",
    )
    stage3h_to_object_wire_contracts = (
        (
            "compiler-natural raw111 provenance",
            stage3h_natural_to_object_wire,
            56,
            0x65A8B3D0D7ED115A,
            "f5bdac14901bb6b752e2ca10a01dd31d6990456c43f78d5923b1da4a0ef3706e",
            bytes.fromhex("0c43020100010001020000000d0100010000"),
            bytes.fromhex("ea06116f0eea04cfeaf90ecf28"),
        ),
        (
            "mechanically derived property-free raw111",
            stage3h_manual_to_object_wire,
            46,
            0xC84F87720CD09B16,
            "13f81e66520578393a57f3290636d4778c5cae8d014591e5daaaacdd3ffd5c95",
            bytes.fromhex("0c4302010001000101000000030100010000"),
            bytes.fromhex("cf6f28"),
        ),
    )
    for label, wire, size, fnv, sha256, metadata, child_code in stage3h_to_object_wire_contracts:
        if (
            len(wire) != size
            or stage3e_fnv1a64(wire) != fnv
            or hashlib.sha256(wire).hexdigest() != sha256
            or wire[25:43] != metadata
            or not wire.endswith(child_code)
        ):
            fail(
                "stage3h-runtime-evidence",
                f"the Rust {label} fixture must retain its exact byte identity, FNV-1a-64, SHA-256, flags/frame metadata, code offset, and child body",
            )

    stage3i_push_this_wire_contracts = (
        (
            "compiler-natural strict raw8",
            stage3e_byte_array(stage3e_runtime_source, "QUICKJS_NATURAL_STRICT_PUSH_THIS_BC5", "stage3i-runtime-evidence"),
            47,
            0x4EC7E0187375D810,
            "786376192d5bfe7eb07115f62788707619ee54e8721acfa66dae1d110a580e39",
            bytes.fromhex("0c4302010000010001000000040100010000"),
            bytes.fromhex("08c7c328"),
        ),
        (
            "compiler-natural sloppy raw8",
            stage3e_byte_array(stage3e_runtime_source, "QUICKJS_NATURAL_SLOPPY_PUSH_THIS_BC5", "stage3i-runtime-evidence"),
            47,
            0x4E7F8F98ADFF8463,
            "f0430a7c241caaf94703bd5de73289d4f90fea3ee9cfaf22a660ed80df3de0a6",
            bytes.fromhex("0c4302000000010001000000040100010000"),
            bytes.fromhex("08c7c328"),
        ),
        (
            "property-free strict raw8",
            stage3e_byte_array(stage3e_runtime_source, "QUICKJS_ORDINARY_STRICT_PUSH_THIS_BC5", "stage3i-runtime-evidence"),
            41,
            0x3C3E393FEF883BC5,
            "9b14c5245a78e0a069967089cf6f89aefac3e12749d16eba36e4c15b72a3c99e",
            bytes.fromhex("0c43020100000000010000000200"),
            bytes.fromhex("0828"),
        ),
        (
            "property-free sloppy raw8",
            stage3e_byte_array(stage3e_runtime_source, "QUICKJS_ORDINARY_SLOPPY_PUSH_THIS_BC5", "stage3i-runtime-evidence"),
            41,
            0x0E2485C97EEA9CFA,
            "213b3b6a332d4cf69e4c726b372c1f0087e70fc9c263a6a2193ce4763fb62648",
            bytes.fromhex("0c43020000000000010000000200"),
            bytes.fromhex("0828"),
        ),
        (
            "duplicate raw8 mismatch",
            stage3e_byte_array(stage3e_runtime_source, "QUICKJS_DUPLICATE_PUSH_THIS_BC5", "stage3i-runtime-evidence"),
            43,
            0x920DE09AAF63833E,
            "9f0541bfd8a599e5f2575936d24df9a2487a1e8952fca1648afeef5c9f798a30",
            bytes.fromhex("0c43020000000000020000000400"),
            bytes.fromhex("0808a928"),
        ),
        (
            "raw8 re-entry mismatch",
            stage3e_byte_array(stage3e_runtime_source, "QUICKJS_REENTER_PUSH_THIS_BC5", "stage3i-runtime-evidence"),
            65,
            0xFA100FF2B0854673,
            "32b4c9e45f5191d21aa44d3437c54b00cfa1ff4b2530d1e4cdf942a87e8f3fb4",
            bytes.fromhex("0c430200000200020200000012020001000000010000"),
            bytes.fromhex("08cf690c0000000ad3d46af5ffffffd0a928"),
        ),
    )
    for label, wire, size, fnv, sha256, metadata, child_code in stage3i_push_this_wire_contracts:
        if (
            len(wire) != size
            or stage3e_fnv1a64(wire) != fnv
            or hashlib.sha256(wire).hexdigest() != sha256
            or wire[25:25 + len(metadata)] != metadata
            or not wire.endswith(child_code)
        ):
            fail(
                "stage3i-runtime-evidence",
                f"the Rust {label} fixture must retain its exact byte identity, FNV-1a-64, SHA-256, flags/frame metadata, code offset, and child body",
            )

    stage3d_test_contracts = (
        (
            "src/runtime/binary_object/function_translate/capability.rs",
            "ordinary_throw_rows_are_the_exact_reviewed_completion_set",
            (
                "CAPABILITY_REGISTRY[48].policy",
                "CapabilityPolicy::OrdinaryOnly(Recipe::Throw)",
                "CAPABILITY_REGISTRY[47].policy",
                "CapabilityPolicy::OrdinaryOnly(Recipe::ThrowReadOnly)",
            ),
        ),
        (
            "src/runtime/binary_object/function_translate/mod.rs",
            "explicit_throw_lowering_is_typed_and_operand_free",
            (
                "lower_operation(Recipe::Throw, &NativeOperands::None).unwrap()",
                "Some(PendingOperation::Ready(FunctionOp::Throw))",
                "assert!(operations.next().is_none());",
            ),
        ),
        (
            "src/runtime/binary_object/ordinary_leaf.rs",
            "lowers_representative_sanitized_operations_without_consulting_diagnostics",
            ("(FunctionOp::Throw, OrdinaryLeafOp::Throw)",),
        ),
        (
            "src/runtime/binary_object_publish.rs",
            "ordinary_leaf_draft_ops_lower_one_for_one_without_reordering",
            (
                "lower(OrdinaryLeafOp::Throw)",
                "Instruction::Throw",
            ),
        ),
        (
            "src/bytecode.rs",
            "verifier_allows_terminal_completion_to_abandon_switch_values",
            (
                "for completion in [Instruction::Return, Instruction::Throw]",
                "assert_eq!(function.verify().unwrap().max_stack, 2);",
            ),
        ),
        (
            "src/runtime/tests.rs",
            "trusted_quickjs_ordinary_throw_uses_the_exact_wire_metadata_and_value_identity",
            (
                "assert_eq!(QUICKJS_ORDINARY_THROW_BC5.len(), 45);",
                "0x73cf_217e_06c5_fee2",
                "[Instruction::GetArg(0), Instruction::Throw]",
                "snapshot.metadata.function_kind, FunctionKind::Normal",
                "Some(Value::Object(object.clone()))",
                "Some(Value::Int(42))",
            ),
        ),
        (
            "src/runtime/tests.rs",
            "trusted_quickjs_ordinary_throw_reenters_caller_catch_backtrace_and_iterator_close",
            (
                "eval_with_filename(",
                "Value::Bool(true)",
                "let stack = own_stack_string(&runtime, &error).to_utf8_lossy();",
                "JsString::from_static(",
                "assert!(!context.has_exception());",
            ),
        ),
        (
            "src/runtime/tests.rs",
            "trusted_quickjs_ordinary_throw_is_terminal_and_branch_targetable",
            (
                "quickjs_ordinary_one_argument_with_code(&[0xcf, 0x30, 0x0e], 1)",
                "Instruction::Throw, Instruction::Drop",
                "quickjs_ordinary_one_argument_with_code(&[0xcf, 0xea, 0x02, 0xb4, 0x30], 1)",
                "Instruction::Goto(3)",
                "Some(Value::Object(object))",
            ),
        ),
        (
            "src/runtime/tests.rs",
            "trusted_quickjs_ordinary_throw_verification_rejects_transactionally_and_retries",
            (
                "quickjs_ordinary_one_argument_with_code(code, max_stack)",
                "assert_eq!(runtime.heap_counts(), baseline",
                "assert_eq!(runtime.test_atom_count(), baseline_atoms",
                "read_trusted_ordinary_function(QUICKJS_ORDINARY_THROW_BC5, 0)",
                "Some(Value::Int(42))",
            ),
        ),
        (
            "src/runtime/tests.rs",
            "trusted_quickjs_ordinary_throw_rejects_nonordinary_metadata_transactionally",
            (
                "async_function[28] |= 1 << 2;",
                "generator[26] |= 1 << 4;",
                "derived[26] |= 1 << 2;",
                "&image[43..], [0xcf, 0x30]",
                ".read_trusted_ordinary_function(&image, 0)",
                "assert_eq!(runtime.heap_counts(), baseline",
                "read_trusted_ordinary_function(QUICKJS_ORDINARY_THROW_BC5, 0)",
                "Some(Value::Int(42))",
            ),
        ),
    )
    stage3d_test_body_hashes = {
        ("src/runtime/binary_object/function_translate/capability.rs", "ordinary_throw_rows_are_the_exact_reviewed_completion_set"): "46280b952611f2513c2764859dacaa8b0b2be02d86a0ffbf92a0d5791e7deb6a",
        ("src/runtime/binary_object/function_translate/mod.rs", "explicit_throw_lowering_is_typed_and_operand_free"): "5d6d1b5266a7b7682c26a39d8b54c4b9f450670feb7dc917e89427293f92b9a9",
        ("src/runtime/binary_object/ordinary_leaf.rs", "lowers_representative_sanitized_operations_without_consulting_diagnostics"): "0da974adaa4d87c8aa9949f1ab1ab764b6408aafcf426d3f3218d426965a697d",
        ("src/runtime/binary_object_publish.rs", "ordinary_leaf_draft_ops_lower_one_for_one_without_reordering"): "d00312170558a488c86b0eb21eeea031f155e0ddb9087e5f3219619282f8706d",
        ("src/bytecode.rs", "verifier_allows_terminal_completion_to_abandon_switch_values"): "7fdb137db6ab6bedd70a5ea42b6b267ab3d1eb027ea6467e25b430ebf365b4a6",
        ("src/runtime/tests.rs", "trusted_quickjs_ordinary_throw_uses_the_exact_wire_metadata_and_value_identity"): "f3df4f2957a2d447ff0f0c1af7c0cb74eb03017dda72210b1349f16d4dceccfb",
        ("src/runtime/tests.rs", "trusted_quickjs_ordinary_throw_reenters_caller_catch_backtrace_and_iterator_close"): "71ae492da72febf0aae78f64edd2980b70982f2d0799981b4e06d336a03eaa87",
        ("src/runtime/tests.rs", "trusted_quickjs_ordinary_throw_is_terminal_and_branch_targetable"): "54f139dbfa37ab0fc7b8e6f2ae09a9246d61fd2ebffdb055d86c3ccdcd1067ef",
        ("src/runtime/tests.rs", "trusted_quickjs_ordinary_throw_verification_rejects_transactionally_and_retries"): "a84862beab3d71440a66f2fd35e7dfaabe35d668b1ae51831376636b7d1cd156",
        ("src/runtime/tests.rs", "trusted_quickjs_ordinary_throw_rejects_nonordinary_metadata_transactionally"): "5c92b46b6a9c05b16802f6ff26de4fef8e5ad509e6e15763f46d62e2f9dc8d27",
    }
    stage3d_inline_test_files = {
        relative
        for relative, _, _ in stage3d_test_contracts
        if relative != "src/runtime/tests.rs"
    }
    stage3d_test_parent_bounds: dict[str, tuple[int, int]] = {}
    for relative in stage3d_inline_test_files:
        code = stage3b_code(relative)
        modules = list(re.finditer(
            r"(?P<attributes>(?:#[ \t\n]*\[[^]]*\][ \t\n]*)*)"
            r"\bmod[ \t\n]+tests[ \t\n]*\{",
            code,
        ))
        if (
            len(modules) != 1
            or " ".join(modules[0].group("attributes").split()) != "#[cfg(test)]"
        ):
            fail(
                "stage3d-runtime-evidence",
                f"{relative} must retain one direct, unconditional #[cfg(test)] tests module",
            )
            continue
        _, module_start, module_end = braced_item_from_match(
            code,
            modules[0],
            "stage3d-runtime-evidence",
            f"{relative} direct tests module",
        )
        stage3d_test_parent_bounds[relative] = (module_start, module_end)

    assertion_shadow = assertion_shadow_pattern
    for relative in stage3d_inline_test_files | {"src/runtime/tests.rs"}:
        if assertion_shadow.search(stage3b_code(relative)):
            fail(
                "stage3d-runtime-evidence",
                f"{relative} must not shadow or import the assertion macros used by Stage3D evidence",
            )

    missing_stage3d_tests = []
    drifted_stage3d_tests = []
    for relative, name, anchors in stage3d_test_contracts:
        code = stage3b_code(relative)
        declarations = list(re.finditer(
            rf"(?P<attributes>(?:#[ \t\n]*\[[^]]*\][ \t\n]*)*)"
            rf"\bfn[ \t\n]+{re.escape(name)}[ \t\n]*\(",
            code,
        ))
        if (
            len(declarations) != 1
            or " ".join(declarations[0].group("attributes").split()) != "#[test]"
        ):
            missing_stage3d_tests.append(f"{relative}::{name}")
            continue
        declaration_offset = declarations[0].start()
        declaration_depth = (
            code[:declaration_offset].count("{")
            - code[:declaration_offset].count("}")
        )
        if relative == "src/runtime/tests.rs":
            direct_parent = declaration_depth == 0
        else:
            parent_bounds = stage3d_test_parent_bounds.get(relative)
            direct_parent = (
                parent_bounds is not None
                and parent_bounds[0] < declaration_offset < parent_bounds[1]
                and declaration_depth == 1
            )
        if not direct_parent:
            missing_stage3d_tests.append(f"{relative}::{name} (nested)")
            continue
        item = stage3b_function(relative, name, "stage3d-runtime-evidence")
        normalized_item = " ".join(item.split())
        if (
            any(anchor not in normalized_item for anchor in anchors)
            or normalized_code_sha256(item)
            != stage3d_test_body_hashes.get((relative, name))
        ):
            drifted_stage3d_tests.append(f"{relative}::{name}")
    if missing_stage3d_tests or drifted_stage3d_tests:
        fail(
            "stage3d-runtime-evidence",
            "Stage3D tests must remain unconditional #[test] functions with the exact raw48 wire, typed chain, terminal verifier, identity, pending, catch, backtrace, iterator-close, recovery, and rollback evidence; "
            f"missing {missing_stage3d_tests}, drifted {drifted_stage3d_tests}",
        )

    stage3e_test_contracts = (
        (
            "src/runtime/binary_object/function_translate/capability.rs",
            "ordinary_throw_rows_are_the_exact_reviewed_completion_set",
            (
                "CAPABILITY_REGISTRY[48].policy",
                "CapabilityPolicy::OrdinaryOnly(Recipe::Throw)",
                "CAPABILITY_REGISTRY[49].policy",
                "CapabilityPolicy::OrdinaryOnly(Recipe::ThrowReadOnly)",
            ),
        ),
        (
            "src/runtime/binary_object/ordinary_leaf.rs",
            "lowers_property_free_read_only_with_owned_input_atom_spelling",
            (
                "assert_eq!(object.len(), 47);",
                "assert_eq!(draft.metadata().max_stack(), 0);",
                "[OrdinaryLeafOp::ThrowReadOnly(name)]",
                "name.0.as_ref()",
            ),
        ),
        (
            "src/runtime/binary_object/ordinary_leaf.rs",
            "read_only_rejects_other_subtypes_non_string_atoms_and_atom_table_drift",
            (
                "for subtype in [1, 2, 3, 4, 5, u8::MAX]",
                "object[46] = subtype;",
                "object[42..46].copy_from_slice(&raw_atom.to_le_bytes());",
                "unused[39] = 1;",
                "multiple[1] = 2;",
            ),
        ),
        (
            "src/runtime/binary_object/ordinary_leaf.rs",
            "read_only_accepts_only_string_names_under_zero_or_one_slot_provenance",
            (
                "predefined[1] = 0;",
                "predefined[40..44].copy_from_slice(&50_u32.to_le_bytes());",
                "manifest_alias.splice(2..4",
                "decimal_alias.splice(2..4",
            ),
        ),
        (
            "src/runtime/binary_object/ordinary_leaf.rs",
            "natural_read_only_wire_remains_outside_the_nonlexical_leaf_cohort",
            (
                "assert_eq!(object.len(), 58);",
                "Err(OrdinaryLeafReadError::Unadmitted(_))",
            ),
        ),
        (
            "src/bytecode.rs",
            "verifier_rejects_bad_constants_and_stack_joins",
            (
                "Instruction::PostInc, Instruction::ThrowReadOnly(0)",
                "let empty_stack_readonly = BytecodeFunction",
                "code: vec![Instruction::ThrowReadOnly(0)]",
                "assert_eq!(empty_stack_readonly.verify().unwrap().max_stack, 0);",
            ),
        ),
        (
            "src/runtime/tests.rs",
            "trusted_quickjs_ordinary_read_only_uses_exact_zero_stack_wire_and_type_error",
            (
                "assert_eq!(QUICKJS_ORDINARY_READ_ONLY_BC5.len(), 47);",
                "0xb4c1_126c_2830_93af",
                "[Instruction::ThrowReadOnly(0)]",
                "snapshot.metadata.max_stack, 0",
                "[BytecodeConstant::Value(RawValue::String(name))]",
                "Err(RuntimeError::Exception)",
            ),
        ),
        (
            "src/runtime/tests.rs",
            "trusted_quickjs_ordinary_read_only_reenters_catch_and_resets_pending_state",
            (
                "read_trusted_ordinary_function(QUICKJS_ORDINARY_READ_ONLY_BC5, 0)",
                "context .define_own_property(",
                "assert!(!context.has_exception());",
            ),
        ),
        (
            "src/runtime/tests.rs",
            "trusted_quickjs_ordinary_read_only_uses_bytecode_realm_and_attaches_backtrace",
            (
                "let mut defining = runtime.new_context();",
                "let mut caller = runtime.new_context();",
                "runtime.get_prototype_of(&error).unwrap()",
                "own_stack_string(&runtime, &error)",
                "assert!(!caller.has_exception());",
            ),
        ),
        (
            "src/runtime/tests.rs",
            "trusted_quickjs_ordinary_read_only_preserves_wide_lone_surrogate_name_units",
            (
                "image.splice(2..4, [0x05, 0x41, 0x00, 0x00, 0xd8]);",
                ".utf16_units() .collect::<Vec<_>>()",
                "0xd800",
                "assert!(!context.has_exception());",
            ),
        ),
        (
            "src/runtime/tests.rs",
            "trusted_quickjs_ordinary_read_only_rejections_are_transactional_and_retryable",
            (
                "subtype_one[46] = 1;",
                "subtype_max[46] = u8::MAX;",
                "unused_atom[39] = 1;",
                "multiple_atoms[1] = 2;",
                "non_string[42..46].copy_from_slice(&0x8000_002a_u32.to_le_bytes());",
                "QUICKJS_NATURAL_READ_ONLY_BC5.to_vec()",
                "assert_eq!(runtime.heap_counts(), baseline",
                "assert_eq!(runtime.test_atom_count(), baseline_atoms",
            ),
        ),
    )
    stage3e_test_body_hashes = {
        ("src/runtime/binary_object/function_translate/capability.rs", "ordinary_throw_rows_are_the_exact_reviewed_completion_set"): "46280b952611f2513c2764859dacaa8b0b2be02d86a0ffbf92a0d5791e7deb6a",
        ("src/runtime/binary_object/ordinary_leaf.rs", "lowers_property_free_read_only_with_owned_input_atom_spelling"): "bc88a01461e1c7a8fc3d05e716510a81eb95b464dc48165b863b86928f970172",
        ("src/runtime/binary_object/ordinary_leaf.rs", "read_only_rejects_other_subtypes_non_string_atoms_and_atom_table_drift"): "e03485e5228e1e346c83d40e2285f09283f71c94f53542af394c04a1e6b4b7aa",
        ("src/runtime/binary_object/ordinary_leaf.rs", "read_only_accepts_only_string_names_under_zero_or_one_slot_provenance"): "2c730d38b8a09aa20b2ba20add47bde755090f23a8599e8516e964352f32eff2",
        ("src/runtime/binary_object/ordinary_leaf.rs", "natural_read_only_wire_remains_outside_the_nonlexical_leaf_cohort"): "b50006a970fd18ce24274af96ee39394e04ae06bf27fa988682dcc3b1480741d",
        ("src/bytecode.rs", "verifier_rejects_bad_constants_and_stack_joins"): "dae9279e19da751453eaec47b74b470d0ee627b0e2a03a1c8fae7c5405907aef",
        ("src/runtime/tests.rs", "trusted_quickjs_ordinary_read_only_uses_exact_zero_stack_wire_and_type_error"): "29f9c54b5ccffea7e9bf100206ccaf0a99b2f4afe8d2ff887ad99b5b11989493",
        ("src/runtime/tests.rs", "trusted_quickjs_ordinary_read_only_reenters_catch_and_resets_pending_state"): "756aa7b8180452d3786a16f1c2534135381e1bd4931458060ff3a5b2b6f39098",
        ("src/runtime/tests.rs", "trusted_quickjs_ordinary_read_only_uses_bytecode_realm_and_attaches_backtrace"): "fd9907b3f5987005582a5376b4305bf68cd74513bd1e46ffd59bd2933495d4d6",
        ("src/runtime/tests.rs", "trusted_quickjs_ordinary_read_only_preserves_wide_lone_surrogate_name_units"): "8e220ef941cc43fa3d487827a94b1298047fd7cea6082df622788d4081b47110",
        ("src/runtime/tests.rs", "trusted_quickjs_ordinary_read_only_rejections_are_transactional_and_retryable"): "a896b877f9b5b0d49ed423701344c738937e055112695c1b264aea9ec729995a",
    }
    missing_stage3e_tests = []
    drifted_stage3e_tests = []
    for relative, name, anchors in stage3e_test_contracts:
        code = stage3b_code(relative)
        declarations = list(re.finditer(
            rf"(?P<attributes>(?:#[ \t\n]*\[[^]]*\][ \t\n]*)*)"
            rf"\bfn[ \t\n]+{re.escape(name)}[ \t\n]*\(",
            code,
        ))
        if (
            len(declarations) != 1
            or " ".join(declarations[0].group("attributes").split()) != "#[test]"
        ):
            missing_stage3e_tests.append(f"{relative}::{name}")
            continue
        declaration_offset = declarations[0].start()
        declaration_depth = code[:declaration_offset].count("{") - code[:declaration_offset].count("}")
        if relative == "src/runtime/tests.rs":
            direct_parent = declaration_depth == 0
        else:
            parent_bounds = stage3d_test_parent_bounds.get(relative)
            direct_parent = (
                parent_bounds is not None
                and parent_bounds[0] < declaration_offset < parent_bounds[1]
                and declaration_depth == 1
            )
        if not direct_parent:
            missing_stage3e_tests.append(f"{relative}::{name} (nested)")
            continue
        item = stage3b_function(relative, name, "stage3e-runtime-evidence")
        normalized_item = " ".join(item.split())
        if (
            any(anchor not in normalized_item for anchor in anchors)
            or normalized_code_sha256(item)
            != stage3e_test_body_hashes.get((relative, name))
        ):
            drifted_stage3e_tests.append(f"{relative}::{name}")
    if missing_stage3e_tests or drifted_stage3e_tests:
        fail(
            "stage3e-runtime-evidence",
            "Stage3E tests must remain unconditional direct-parent #[test] functions with exact raw49 wires, subtype/atom provenance, synthetic String, zero-stack terminal, realm, backtrace, pending, catch, retry, and rollback evidence; "
            f"missing {missing_stage3e_tests}, drifted {drifted_stage3e_tests}",
        )

    stage3f_test_contracts = (
        (
            "src/runtime/binary_object/function_translate/capability.rs",
            "ordinary_nop_is_the_exact_operand_free_raw177_row",
            (
                "CAPABILITY_REGISTRY[177]",
                "OpcodeFormat::None",
                "CapabilityPolicy::OrdinaryOnly(Recipe::Nop)",
            ),
        ),
        (
            "src/runtime/binary_object/function_translate/mod.rs",
            "operand_free_nop_translation_is_one_typed_operation",
            (
                "lower_operation(Recipe::Nop, &NativeOperands::None).unwrap()",
                "Some(PendingOperation::Ready(FunctionOp::Nop))",
                "TranslationTarget::Scalar",
                "Some(PendingOperation::Ready(FunctionOp::OutsideTarget))",
            ),
        ),
        (
            "src/runtime/binary_object/ordinary_leaf.rs",
            "lowers_representative_sanitized_operations_without_consulting_diagnostics",
            ("(FunctionOp::Nop, OrdinaryLeafOp::Nop)",),
        ),
        (
            "src/runtime/binary_object_publish.rs",
            "ordinary_leaf_draft_ops_lower_one_for_one_without_reordering",
            ("lower(OrdinaryLeafOp::Nop)", "Instruction::Nop"),
        ),
        (
            "src/bytecode.rs",
            "verifier_accepts_closed_non_terminating_control_flow",
            (
                "code: vec![Instruction::Nop]",
                "reachable_fallthrough.verify().unwrap_err().message()",
            ),
        ),
        (
            "src/runtime/tests.rs",
            "trusted_quickjs_ordinary_nop_preserves_exact_metadata_realm_and_zero_effect",
            (
                "assert_eq!(QUICKJS_ORDINARY_NOP_BC5.len(), 41);",
                "0x1c52_2736_e3cb_ef92",
                "[Instruction::Nop, Instruction::ReturnUndefined]",
                "snapshot.constants.is_empty()",
                "snapshot.metadata.max_stack, 0",
                "Some(defining.function_prototype().unwrap())",
                "for _ in 0..2",
                "assert!(!caller.has_exception());",
            ),
        ),
        (
            "src/runtime/tests.rs",
            "trusted_quickjs_ordinary_nop_only_fallthrough_rolls_back_and_retries",
            (
                "nop_only[37] = 1;",
                "nop_only.truncate(40);",
                "nop_only[39], 0xb1",
                "error.message(), FALLTHROUGH",
                "assert_eq!(runtime.heap_counts(), baseline",
                "assert_eq!(runtime.test_atom_count(), baseline_atoms",
                "read_trusted_ordinary_function(QUICKJS_ORDINARY_NOP_BC5, 0)",
                "assert!(!context.has_exception());",
            ),
        ),
        (
            "src/runtime/tests.rs",
            "trusted_quickjs_ordinary_branch_targets_raw177_typed_index",
            (
                "image.extend_from_slice(&[0xea, 0x01, 0xb1, 0x29]);",
                "Instruction::Goto(1)",
                "Instruction::Nop",
                "Instruction::ReturnUndefined",
                "assert!(!context.has_exception());",
            ),
        ),
    )
    stage3f_test_body_hashes = {
        ("src/runtime/binary_object/function_translate/capability.rs", "ordinary_nop_is_the_exact_operand_free_raw177_row"): "f75d3876877204f9b2a209216e5664095bab427ab1831db8ae7fdda946362b17",
        ("src/runtime/binary_object/function_translate/mod.rs", "operand_free_nop_translation_is_one_typed_operation"): "3fdd3855d14295ac08d90c15878eeb3a17afb2e9d5a6008182a0b0193cbd6ae4",
        ("src/runtime/binary_object/ordinary_leaf.rs", "lowers_representative_sanitized_operations_without_consulting_diagnostics"): "0da974adaa4d87c8aa9949f1ab1ab764b6408aafcf426d3f3218d426965a697d",
        ("src/runtime/binary_object_publish.rs", "ordinary_leaf_draft_ops_lower_one_for_one_without_reordering"): "d00312170558a488c86b0eb21eeea031f155e0ddb9087e5f3219619282f8706d",
        ("src/bytecode.rs", "verifier_accepts_closed_non_terminating_control_flow"): "7b249cca44a54c95013a2119764943705c06e0c372483c50b3d46ee41dd2d717",
        ("src/runtime/tests.rs", "trusted_quickjs_ordinary_nop_preserves_exact_metadata_realm_and_zero_effect"): "bab4e09fe90b483999fbe41f9a1bbf26724ce74834fa2faba1d8a1ece16bce82",
        ("src/runtime/tests.rs", "trusted_quickjs_ordinary_nop_only_fallthrough_rolls_back_and_retries"): "110183deebf1bd2e02361b7302b2e033243a57a6b7d51c33ded2919eade36c7c",
        ("src/runtime/tests.rs", "trusted_quickjs_ordinary_branch_targets_raw177_typed_index"): "4e0aa47698b6fd4bfe02e0dde0461251def3229450253d01b03fda139f554345",
    }
    missing_stage3f_tests = []
    drifted_stage3f_tests = []
    for relative, name, anchors in stage3f_test_contracts:
        code = stage3b_code(relative)
        declarations = list(re.finditer(
            rf"(?P<attributes>(?:#[ \t\n]*\[[^]]*\][ \t\n]*)*)"
            rf"\bfn[ \t\n]+{re.escape(name)}[ \t\n]*\(",
            code,
        ))
        if (
            len(declarations) != 1
            or " ".join(declarations[0].group("attributes").split()) != "#[test]"
        ):
            missing_stage3f_tests.append(f"{relative}::{name}")
            continue
        declaration_offset = declarations[0].start()
        declaration_depth = code[:declaration_offset].count("{") - code[:declaration_offset].count("}")
        if relative == "src/runtime/tests.rs":
            direct_parent = declaration_depth == 0
        else:
            parent_bounds = stage3d_test_parent_bounds.get(relative)
            direct_parent = (
                parent_bounds is not None
                and parent_bounds[0] < declaration_offset < parent_bounds[1]
                and declaration_depth == 1
            )
        if not direct_parent:
            missing_stage3f_tests.append(f"{relative}::{name} (nested)")
            continue
        item = stage3b_function(relative, name, "stage3f-runtime-evidence")
        normalized_item = " ".join(item.split())
        if (
            any(anchor not in normalized_item for anchor in anchors)
            or normalized_code_sha256(item)
            != stage3f_test_body_hashes.get((relative, name))
        ):
            drifted_stage3f_tests.append(
                f"{relative}::{name} ({normalized_code_sha256(item)})"
            )
    if missing_stage3f_tests or drifted_stage3f_tests:
        fail(
            "stage3f-runtime-evidence",
            "Stage3F tests must remain unconditional direct-parent #[test] functions with the exact raw177 registry/typed chain, 41-byte runtime metadata/realm/pending evidence, 40-byte fallthrough rollback/retry, and branch-index preservation; "
            f"missing {missing_stage3f_tests}, drifted {drifted_stage3f_tests}",
        )

    stage3i_rust_diff_sha256 = (
        "b45f8479d516fe4c94f0e373258be9ef05c2c07269d36350a24f33c17fb9793b"
    )
    stage3i_rust_file_hashes = {
        "src/runtime/binary_object/function_translate/capability.rs": "d66de6d4f82a70ec689e54124352ed2d9495403a715c4e22fd6914604d8ed7be",
        "src/runtime/binary_object/function_translate/dto.rs": "1f48c15bf4dbc70dc2829f3343bec8cf46aa0330e9b6a8f0cadd2984fd904b0f",
        "src/runtime/binary_object/function_translate/mod.rs": "804236a8dd557718e1c8842aa01cfa3b5187510dcf7bab5b01330b8f59367f36",
        "src/runtime/binary_object/ordinary_leaf.rs": "e73e550e29e57d52918db2ac16e9be3d9da7381947ebab5135c36b102d0bbf53",
        "src/runtime/binary_object_publish.rs": "2774b1bbfd9ffb4d3fc41ea24ffc7abb67605042d5fcd1d6069a6fc105ec08f9",
        "src/runtime/tests.rs": "984f00fde7cd90d1d336b81ba43cda4a7105b99f3b6fdde684d4579fffa9b3bd",
    }
    for relative, expected_hash in stage3i_rust_file_hashes.items():
        path = root / relative
        if path.is_symlink() or not path.is_file():
            fail("stage3i-rust-freeze", f"{relative} must remain a regular frozen Stage3I Rust file")
            continue
        found_hash = hashlib.sha256(path.read_bytes()).hexdigest()
        if found_hash != expected_hash:
            fail(
                "stage3i-rust-freeze",
                f"{relative} drifted from frozen Rust6 diff {stage3i_rust_diff_sha256}; found {found_hash}",
            )

    require_normalized_code_sha256(
        "stage3g-object-translation-route",
        "raw11 Object lowering must remain an operand-free one-operation translation with no alias or expansion",
        stage3b_function(
            "src/runtime/binary_object/function_translate/mod.rs",
            "lower_operation",
            "stage3g-object-translation-route",
        ),
        "25b42c25b4e7b6a14304a4b37456420725fa45dd7f6f145be02b096e2718d2c2",
    )
    require_normalized_code_sha256(
        "stage3g-object-translation-route",
        "translate_native_plan must retain its exact alias-free two-pass publisher so raw11 Object cannot be intercepted, erased, remapped, or index-collapsed before publication",
        translate_native_item,
        "fe149677e125ffef44ebac61b8b9799eff3166eafd7efa93e086b84571eb9867",
    )
    if re.search(
        r"\b(?:Recipe|FunctionOp|OrdinaryLeafOp|Instruction)"
        r"[ \t\n]*::[ \t\n]*Object\b",
        translate_native_item,
    ):
        fail(
            "stage3g-object-translation-route",
            "raw11 Object must not acquire a pre-match, second-pass dispatch, erase, remap, or index-collapse path outside the typed lower_operation handoff",
        )
    require_normalized_code_sha256(
        "stage3g-object-ordinary-route",
        "FunctionOp::Object must reach exactly one OrdinaryLeafOp::Object without erasure or remap",
        stage3b_function(
            "src/runtime/binary_object/ordinary_leaf.rs",
            "lower_operation",
            "stage3g-object-ordinary-route",
        ),
        "77d12d77999176f90897eb647ef49229847c4504475525cdc237b87ba2b33da2",
    )
    require_normalized_code_sha256(
        "stage3g-object-publication",
        "OrdinaryLeafOp::Object must publish exactly Instruction::Object without dropping it or touching the synthetic-constant index",
        stage3b_function(
            "src/runtime/binary_object_publish.rs",
            "lower_ordinary_leaf_op",
            "stage3g-object-publication",
        ),
        "97744dba5220743068ed1afe133603daaf32baf362b361dbb4d9d4efdda4f6c8",
    )

    require_normalized_code_sha256(
        "stage3h-to-object-translation-route",
        "raw111 ToObject lowering must remain an operand-free one-operation translation with no alias or expansion",
        stage3b_function(
            "src/runtime/binary_object/function_translate/mod.rs",
            "lower_operation",
            "stage3h-to-object-translation-route",
        ),
        "25b42c25b4e7b6a14304a4b37456420725fa45dd7f6f145be02b096e2718d2c2",
    )
    require_normalized_code_sha256(
        "stage3h-to-object-translation-route",
        "translate_native_plan must retain its exact alias-free two-pass publisher so raw111 ToObject cannot be intercepted, erased, remapped, or index-collapsed before publication",
        translate_native_item,
        "fe149677e125ffef44ebac61b8b9799eff3166eafd7efa93e086b84571eb9867",
    )
    if re.search(
        r"\b(?:Recipe|FunctionOp|OrdinaryLeafOp|Instruction)"
        r"[ \t\n]*::[ \t\n]*ToObject\b",
        translate_native_item,
    ):
        fail(
            "stage3h-to-object-translation-route",
            "raw111 ToObject must not acquire a pre-match, second-pass dispatch, erase, remap, or index-collapse path outside the typed lower_operation handoff",
        )
    require_normalized_code_sha256(
        "stage3h-to-object-ordinary-route",
        "FunctionOp::ToObject must reach exactly one OrdinaryLeafOp::ToObject without erasure or remap",
        stage3b_function(
            "src/runtime/binary_object/ordinary_leaf.rs",
            "lower_operation",
            "stage3h-to-object-ordinary-route",
        ),
        "77d12d77999176f90897eb647ef49229847c4504475525cdc237b87ba2b33da2",
    )
    require_normalized_code_sha256(
        "stage3h-to-object-publication",
        "OrdinaryLeafOp::ToObject must publish exactly Instruction::ToObject without dropping it or touching the synthetic-constant index",
        stage3b_function(
            "src/runtime/binary_object_publish.rs",
            "lower_ordinary_leaf_op",
            "stage3h-to-object-publication",
        ),
        "97744dba5220743068ed1afe133603daaf32baf362b361dbb4d9d4efdda4f6c8",
    )

    require_normalized_code_sha256(
        "stage3i-push-this-translation-route",
        "raw8 PushThis lowering must remain an operand-free one-operation translation with no recipe or DTO alias",
        stage3b_function(
            "src/runtime/binary_object/function_translate/mod.rs",
            "lower_operation",
            "stage3i-push-this-translation-route",
        ),
        "25b42c25b4e7b6a14304a4b37456420725fa45dd7f6f145be02b096e2718d2c2",
    )
    require_normalized_code_sha256(
        "stage3i-push-this-translation-route",
        "translate_native_plan must retain its exact alias-free source-to-output map so raw8 cannot be intercepted, erased, expanded, remapped, or index-collapsed",
        translate_native_item,
        "fe149677e125ffef44ebac61b8b9799eff3166eafd7efa93e086b84571eb9867",
    )
    if re.search(
        r"\b(?:Recipe|FunctionOp|OrdinaryLeafOp|Instruction)"
        r"[ \t\n]*::[ \t\n]*PushThis\b",
        translate_native_item,
    ):
        fail(
            "stage3i-push-this-translation-route",
            "raw8 PushThis must not acquire a pre-match, second-pass dispatch, erase, remap, or source/output index-collapse path outside the typed lower_operation handoff",
        )

    stage3i_lower_code = stage3b_function(
        "src/runtime/binary_object/ordinary_leaf.rs",
        "lower_code",
        "stage3i-push-this-protocol",
    )
    require_normalized_code_sha256(
        "stage3i-push-this-protocol",
        "ordinary lowering must validate the raw8 entrance protocol before atom accounting, operation lowering, or publication",
        stage3i_lower_code,
        "802efb137202fe4c4b7380359e627b9e97d676ed0e19b6041a58230e03820557",
    )
    require_ordered_fragments(
        "stage3i-push-this-protocol",
        "validate_push_this_protocol(code) must run exactly once before the input-atom ledger and typed output loop",
        stage3i_lower_code,
        (
            "validate_push_this_protocol(code)?;",
            "let mut input_atoms = InputAtomLedger::new(input_atom_slot_count)?;",
            "for instruction in code.instructions() {",
        ),
    )
    stage3i_validator = stage3b_function(
        "src/runtime/binary_object/ordinary_leaf.rs",
        "validate_push_this_protocol",
        "stage3i-push-this-protocol",
    )
    require_normalized_code_sha256(
        "stage3i-push-this-protocol",
        "the raw8 entrance validator must preserve its exact count, typed-index-zero, and explicit-branch-target-zero predicates",
        stage3i_validator,
        "be9784b4c3c11d623428f5be036b0c01445dfc61a7bf32b5ad8d515762fe451d",
    )
    normalized_stage3i_validator = " ".join(stage3i_validator.split())
    stage3i_validator_fragments = (
        "if matches!(instruction.operation(), FunctionOp::PushThis) { push_this_count += 1; push_this_index.get_or_insert(index); }",
        "if push_this_count == 0 { return Ok(()); }",
        "if push_this_count != 1 { return unadmitted(",
        "if push_this_index != Some(0) { return unadmitted(",
        "FunctionOp::IfFalse(0) | FunctionOp::IfTrue(0) | FunctionOp::Goto(0)",
    )
    if any(
        normalized_stage3i_validator.count(fragment) != 1
        for fragment in stage3i_validator_fragments
    ):
        fail(
            "stage3i-push-this-protocol",
            "raw8-absent bodies must remain unchanged, while a present raw8 must occur exactly once at typed index zero and have no explicit IfFalse, IfTrue, or Goto edge back to zero",
        )
    require_normalized_code_sha256(
        "stage3i-push-this-ordinary-route",
        "FunctionOp::PushThis must reach exactly one OrdinaryLeafOp::PushThis without a direct, aliased, helper-mediated, or pre-match bypass",
        stage3b_function(
            "src/runtime/binary_object/ordinary_leaf.rs",
            "lower_operation",
            "stage3i-push-this-ordinary-route",
        ),
        "77d12d77999176f90897eb647ef49229847c4504475525cdc237b87ba2b33da2",
    )
    require_normalized_code_sha256(
        "stage3i-push-this-publication",
        "OrdinaryLeafOp::PushThis must publish exactly Instruction::PushThis without dropping it or consuming a synthetic constant index",
        stage3b_function(
            "src/runtime/binary_object_publish.rs",
            "lower_ordinary_leaf_op",
            "stage3i-push-this-publication",
        ),
        "97744dba5220743068ed1afe133603daaf32baf362b361dbb4d9d4efdda4f6c8",
    )

    stage3g_test_contracts = (
        (
            "src/runtime/binary_object/function_translate/capability.rs",
            "registry_locks_the_current_physical_cohorts",
            (
                "(111, 1, 103, 29)",
                "assert_eq!(scalar_only + shared, 30);",
                "assert_eq!(ordinary_only + shared, 132);",
                "assert_eq!(scalar_only + ordinary_only + shared, 133);",
            ),
        ),
        (
            "src/runtime/binary_object/function_translate/capability.rs",
            "ordinary_object_is_the_exact_operand_free_raw11_row",
            (
                "CAPABILITY_REGISTRY[11]",
                "OpcodeFormat::None",
                "CapabilityPolicy::OrdinaryOnly(Recipe::Object)",
            ),
        ),
        (
            "src/runtime/binary_object/function_translate/capability.rs",
            "blocked_frontier_has_stable_typed_category_counts",
            (
                "[1, 5, 2, 1, 3, 7, 16, 15, 25, 4, 9, 11, 5, 4, 3]",
                "counts.into_iter().sum::<usize>(), 111",
            ),
        ),
        (
            "src/runtime/binary_object/function_translate/mod.rs",
            "operand_free_object_translation_is_one_ordinary_typed_operation",
            (
                "lower_operation(Recipe::Object, &NativeOperands::None).unwrap()",
                "Some(PendingOperation::Ready(FunctionOp::Object))",
                "TranslationTarget::Scalar",
                "Some(PendingOperation::Ready(FunctionOp::OutsideTarget))",
            ),
        ),
        (
            "src/runtime/binary_object/ordinary_leaf.rs",
            "lowers_representative_sanitized_operations_without_consulting_diagnostics",
            ("(FunctionOp::Object, OrdinaryLeafOp::Object)",),
        ),
        (
            "src/runtime/binary_object_publish.rs",
            "ordinary_leaf_draft_ops_lower_one_for_one_without_reordering",
            ("lower(OrdinaryLeafOp::Object)", "Instruction::Object"),
        ),
        (
            "src/runtime/tests.rs",
            "trusted_quickjs_ordinary_object_is_natural_fresh_and_defining_realm_owned",
            (
                "assert_eq!(QUICKJS_ORDINARY_OBJECT_BC5.len(), 41);",
                "0x3c41_af3f_ef8b_3a1e",
                "[Instruction::Object, Instruction::Return]",
                "snapshot.metadata.max_stack, 1",
                "Some(defining_object_prototype.clone())",
                "assert_ne!(object, other",
                "assert!(!defining.has_exception());",
                "assert!(!caller.has_exception());",
            ),
        ),
        (
            "src/runtime/tests.rs",
            "trusted_quickjs_ordinary_object_verification_rolls_back_and_retries",
            (
                "object_only[37] = 1;",
                "object_only.truncate(40);",
                "undersized_stack[33] = 0;",
                "assert_eq!(runtime.heap_counts(), baseline",
                "assert_eq!(runtime.test_atom_count(), baseline_atoms",
                "read_trusted_ordinary_function(QUICKJS_ORDINARY_OBJECT_BC5, 0)",
                "assert!(!context.has_exception());",
            ),
        ),
        (
            "src/runtime/tests.rs",
            "trusted_quickjs_ordinary_branch_targets_raw11_typed_index",
            (
                "image.extend_from_slice(&[0xea, 0x01, 0x0b, 0x28]);",
                "Instruction::Goto(1)",
                "Instruction::Object",
                "Instruction::Return",
                "assert_ne!(first, second);",
                "Some(object_prototype.clone())",
                "assert!(!context.has_exception());",
            ),
        ),
    )
    stage3g_test_body_hashes = {
        ("src/runtime/binary_object/function_translate/capability.rs", "registry_locks_the_current_physical_cohorts"): "3ff8521f17f3acd991542c7de3e769507ba061eb19d605a413e4febe65f77cc9",
        ("src/runtime/binary_object/function_translate/capability.rs", "ordinary_object_is_the_exact_operand_free_raw11_row"): "7f4c0d8c2f74ecd1294d184dd2897046a3f68b19f37d74d88e80cb90d14f2960",
        ("src/runtime/binary_object/function_translate/capability.rs", "blocked_frontier_has_stable_typed_category_counts"): "601aa4392300e4a3f965cef80787945d4aea706fdc729325318a016e17bb41c8",
        ("src/runtime/binary_object/function_translate/mod.rs", "operand_free_object_translation_is_one_ordinary_typed_operation"): "723c1ada6b0fd572d80fef51ebe969b0a0b4921d12905fd41a27599b10f805bd",
        ("src/runtime/binary_object/ordinary_leaf.rs", "lowers_representative_sanitized_operations_without_consulting_diagnostics"): "0da974adaa4d87c8aa9949f1ab1ab764b6408aafcf426d3f3218d426965a697d",
        ("src/runtime/binary_object_publish.rs", "ordinary_leaf_draft_ops_lower_one_for_one_without_reordering"): "d00312170558a488c86b0eb21eeea031f155e0ddb9087e5f3219619282f8706d",
        ("src/runtime/tests.rs", "trusted_quickjs_ordinary_object_is_natural_fresh_and_defining_realm_owned"): "e95d3b4621ba76bef9b61ec1e888ced91fd8da56b6140769577537e887049734",
        ("src/runtime/tests.rs", "trusted_quickjs_ordinary_object_verification_rolls_back_and_retries"): "d7926f01d7c36d50c331ce42b462e658156eb1f2cae0f3ada3c87fcd92ffd5da",
        ("src/runtime/tests.rs", "trusted_quickjs_ordinary_branch_targets_raw11_typed_index"): "a787e966b4682fc3753b44657acd7b69c76d6684e7731347a031ca4dcc917635",
    }
    missing_stage3g_tests = []
    drifted_stage3g_tests = []
    for relative, name, anchors in stage3g_test_contracts:
        code = stage3b_code(relative)
        declarations = list(re.finditer(
            rf"(?P<attributes>(?:#[ \t\n]*\[[^]]*\][ \t\n]*)*)"
            rf"\bfn[ \t\n]+{re.escape(name)}[ \t\n]*\(",
            code,
        ))
        if (
            len(declarations) != 1
            or " ".join(declarations[0].group("attributes").split()) != "#[test]"
        ):
            missing_stage3g_tests.append(f"{relative}::{name}")
            continue
        declaration_offset = declarations[0].start()
        declaration_depth = code[:declaration_offset].count("{") - code[:declaration_offset].count("}")
        if relative == "src/runtime/tests.rs":
            direct_parent = declaration_depth == 0
        else:
            parent_bounds = stage3d_test_parent_bounds.get(relative)
            direct_parent = (
                parent_bounds is not None
                and parent_bounds[0] < declaration_offset < parent_bounds[1]
                and declaration_depth == 1
            )
        if not direct_parent:
            missing_stage3g_tests.append(f"{relative}::{name} (nested)")
            continue
        item = stage3b_function(relative, name, "stage3g-runtime-evidence")
        normalized_item = " ".join(item.split())
        item_hash = normalized_code_sha256(item)
        if (
            any(anchor not in normalized_item for anchor in anchors)
            or item_hash != stage3g_test_body_hashes.get((relative, name))
        ):
            drifted_stage3g_tests.append(f"{relative}::{name} ({item_hash})")
    if missing_stage3g_tests or drifted_stage3g_tests:
        fail(
            "stage3g-runtime-evidence",
            "Stage3G tests must remain unconditional direct-parent #[test] functions with exact counts/blocker vector, raw11 typed handoff, 41-byte natural wire/max-stack/realm/freshness, transactional rollback/retry, and branch-index evidence; "
            f"missing {missing_stage3g_tests}, drifted {drifted_stage3g_tests}",
        )

    stage3h_test_contracts = (
        (
            "src/runtime/binary_object/function_translate/capability.rs",
            "registry_locks_the_current_physical_cohorts",
            (
                "(111, 1, 103, 29)",
                "assert_eq!(scalar_only + shared, 30);",
                "assert_eq!(ordinary_only + shared, 132);",
                "assert_eq!(scalar_only + ordinary_only + shared, 133);",
            ),
        ),
        (
            "src/runtime/binary_object/function_translate/capability.rs",
            "ordinary_to_object_is_the_exact_operand_free_raw111_row",
            (
                "CAPABILITY_REGISTRY[111]",
                "OpcodeFormat::None",
                "CapabilityPolicy::OrdinaryOnly(Recipe::ToObject)",
                "opcode.n_pop(), 1",
                "opcode.n_push(), 1",
                "CAPABILITY_REGISTRY[112].policy",
                "CapabilityPolicy::Blocked(TranslationBlocker::ValueConstruction)",
                "CAPABILITY_REGISTRY[47].policy",
                "CapabilityPolicy::Blocked(TranslationBlocker::Completion)",
            ),
        ),
        (
            "src/runtime/binary_object/function_translate/capability.rs",
            "blocked_frontier_has_stable_typed_category_counts",
            (
                "[1, 5, 2, 1, 3, 7, 16, 15, 25, 4, 9, 11, 5, 4, 3]",
                "counts.into_iter().sum::<usize>(), 111",
            ),
        ),
        (
            "src/runtime/binary_object/function_translate/mod.rs",
            "operand_free_to_object_translation_is_one_ordinary_typed_operation",
            (
                "lower_operation(Recipe::ToObject, &NativeOperands::None).unwrap()",
                "Some(PendingOperation::Ready(FunctionOp::ToObject))",
                "assert!(operations.next().is_none());",
                "TranslationTarget::Scalar",
                "Some(PendingOperation::Ready(FunctionOp::OutsideTarget))",
            ),
        ),
        (
            "src/runtime/binary_object/ordinary_leaf.rs",
            "lowers_representative_sanitized_operations_without_consulting_diagnostics",
            (("FunctionOp::ToObject, OrdinaryLeafOp::ToObject"),),
        ),
        (
            "src/runtime/binary_object_publish.rs",
            "ordinary_leaf_draft_ops_lower_one_for_one_without_reordering",
            ("lower(OrdinaryLeafOp::ToObject)", "Instruction::ToObject"),
        ),
        (
            "src/runtime/binary_object_publish.rs",
            "ordinary_to_object_publishes_one_for_one_without_a_synthetic_constant",
            (
                "let mut next_synthetic_index = 7;",
                "lower_ordinary_leaf_op(OrdinaryLeafOp::ToObject, &mut next_synthetic_index)",
                "Ok(Instruction::ToObject)",
                "assert_eq!(next_synthetic_index, 7);",
            ),
        ),
        (
            "src/vm.rs",
            "to_object_boxes_primitives_and_rejects_nullish_values",
            (
                "Instruction::ToObject",
                "host.box_primitive_results.push_back(Ok(Value::Int(42)));",
                "host.box_primitive_inputs, [Value::Int(7)]",
                "for nullish in [Instruction::Null, Instruction::Undefined]",
                "error.kind(), ErrorKind::Type",
                "assert!(host.box_primitive_inputs.is_empty());",
            ),
        ),
        (
            "src/runtime/tests.rs",
            "trusted_quickjs_ordinary_to_object_is_natural_exact_and_realm_correct",
            (
                "QUICKJS_NATURAL_TO_OBJECT_BC5.len(), 56",
                "0x65a8_b3d0_d7ed_115a",
                "QUICKJS_ORDINARY_TO_OBJECT_BC5.len(), 46",
                "0xc84f_8772_0cd0_9b16",
                "Instruction::GetArg(0), Instruction::ToObject, Instruction::Return,",
                "snapshot.metadata.max_stack, 1",
                "for (label, primitive, defining_prototype, caller_prototype) in primitive_cases",
                "assert_ne!(first, second",
                "Some(defining_prototype.clone())",
                "same_value(&primitive)",
            ),
        ),
        (
            "src/runtime/tests.rs",
            "trusted_quickjs_ordinary_to_object_nullish_is_pending_and_catchable",
            (
                "for nullish in [Value::Null, Value::Undefined]",
                "Err(RuntimeError::Exception)",
                "assert!(caller.has_exception());",
                "caller.take_exception().unwrap().unwrap()",
                "Some(defining_type_error_prototype.clone())",
                "assert!(!caller.has_exception());",
            ),
        ),
        (
            "src/runtime/tests.rs",
            "trusted_quickjs_ordinary_to_object_verification_rolls_back_and_retries",
            (
                "fallthrough[37] = 2;",
                "fallthrough.truncate(45);",
                "undersized_stack[33] = 0;",
                "assert_eq!(runtime.heap_counts(), baseline",
                "assert_eq!(runtime.test_atom_count(), baseline_atoms",
                "read_trusted_ordinary_function(QUICKJS_ORDINARY_TO_OBJECT_BC5, 0)",
                "assert!(!context.has_exception());",
            ),
        ),
        (
            "src/runtime/tests.rs",
            "trusted_quickjs_ordinary_branch_targets_raw111_typed_index",
            (
                "image.extend_from_slice(&[0xcf, 0xea, 0x01, 0x6f, 0x28]);",
                "Instruction::Goto(2)",
                "Instruction::ToObject",
                "Instruction::Return",
                "Some(boolean_prototype)",
                "assert!(!context.has_exception());",
            ),
        ),
    )
    stage3h_test_body_hashes = {
        ("src/runtime/binary_object/function_translate/capability.rs", "registry_locks_the_current_physical_cohorts"): "3ff8521f17f3acd991542c7de3e769507ba061eb19d605a413e4febe65f77cc9",
        ("src/runtime/binary_object/function_translate/capability.rs", "ordinary_to_object_is_the_exact_operand_free_raw111_row"): "6e473fb62907879029b7701d1e06ccef7e8f37f27f7bd40651595492c11c3dbe",
        ("src/runtime/binary_object/function_translate/capability.rs", "blocked_frontier_has_stable_typed_category_counts"): "601aa4392300e4a3f965cef80787945d4aea706fdc729325318a016e17bb41c8",
        ("src/runtime/binary_object/function_translate/mod.rs", "operand_free_to_object_translation_is_one_ordinary_typed_operation"): "0df04adbb5e0bc9b405351a7d410694b5ef135f40454dddce67c7534c3da73d3",
        ("src/runtime/binary_object/ordinary_leaf.rs", "lowers_representative_sanitized_operations_without_consulting_diagnostics"): "0da974adaa4d87c8aa9949f1ab1ab764b6408aafcf426d3f3218d426965a697d",
        ("src/runtime/binary_object_publish.rs", "ordinary_leaf_draft_ops_lower_one_for_one_without_reordering"): "d00312170558a488c86b0eb21eeea031f155e0ddb9087e5f3219619282f8706d",
        ("src/runtime/binary_object_publish.rs", "ordinary_to_object_publishes_one_for_one_without_a_synthetic_constant"): "f012eb117af444150acf39aae787468928be0b3f34a830ec4817c88323fed383",
        ("src/vm.rs", "to_object_boxes_primitives_and_rejects_nullish_values"): "826673e97da25929d9d4a5ee839db26f9aeffd2c7fe08c986f96e6bd27ae0787",
        ("src/runtime/tests.rs", "trusted_quickjs_ordinary_to_object_is_natural_exact_and_realm_correct"): "45b573dae2ca21be506fc91ba34bc9252aaa71f7b6c0d450b04c1a05a65cd269",
        ("src/runtime/tests.rs", "trusted_quickjs_ordinary_to_object_nullish_is_pending_and_catchable"): "4044acfaecacc5c65c86f2b9cc6a845c5913dc191b6627599368535c56191d6c",
        ("src/runtime/tests.rs", "trusted_quickjs_ordinary_to_object_verification_rolls_back_and_retries"): "e322549d0627a592d693b67ea4225105bd115e48b46d3055f0860c0a60755398",
        ("src/runtime/tests.rs", "trusted_quickjs_ordinary_branch_targets_raw111_typed_index"): "bf79684d2e98f8c48c0f3da07e1f488e1d32116ef919274fc3982b4da92a5bff",
    }
    stage3h_test_parent_bounds = dict(stage3d_test_parent_bounds)
    vm_test_code = stage3b_code("src/vm.rs")
    vm_test_modules = list(re.finditer(
        r"(?P<attributes>(?:#[ \t\n]*\[[^]]*\][ \t\n]*)*)"
        r"\bmod[ \t\n]+tests[ \t\n]*\{",
        vm_test_code,
    ))
    if (
        len(vm_test_modules) != 1
        or " ".join(vm_test_modules[0].group("attributes").split()) != "#[cfg(test)]"
    ):
        fail(
            "stage3h-runtime-evidence",
            "src/vm.rs must retain one direct, unconditional #[cfg(test)] tests module",
        )
    else:
        _, vm_module_start, vm_module_end = braced_item_from_match(
            vm_test_code,
            vm_test_modules[0],
            "stage3h-runtime-evidence",
            "src/vm.rs direct tests module",
        )
        stage3h_test_parent_bounds["src/vm.rs"] = (vm_module_start, vm_module_end)

    stage3h_test_sources = {
        relative for relative, _, _ in stage3h_test_contracts
    }
    for relative in stage3h_test_sources:
        code = stage3b_code(relative)
        if assertion_shadow.search(code):
            fail(
                "stage3h-runtime-evidence",
                f"{relative} must not shadow or import the assertion macros used by Stage3H evidence",
            )
        if re.search(
            r"(?m)^[ \t]*#![ \t]*\[[ \t]*(?:cfg|cfg_attr)\b",
            code,
        ):
            fail(
                "stage3h-runtime-evidence",
                f"{relative} must not conditionally exclude its Stage3H test evidence",
            )

    missing_stage3h_tests = []
    drifted_stage3h_tests = []
    for relative, name, anchors in stage3h_test_contracts:
        code = stage3b_code(relative)
        declarations = list(re.finditer(
            rf"(?P<attributes>(?:#[ \t\n]*\[[^]]*\][ \t\n]*)*)"
            rf"\bfn[ \t\n]+{re.escape(name)}[ \t\n]*\(",
            code,
        ))
        if (
            len(declarations) != 1
            or " ".join(declarations[0].group("attributes").split()) != "#[test]"
        ):
            missing_stage3h_tests.append(f"{relative}::{name}")
            continue
        declaration_offset = declarations[0].start()
        declaration_depth = code[:declaration_offset].count("{") - code[:declaration_offset].count("}")
        if relative == "src/runtime/tests.rs":
            direct_parent = declaration_depth == 0
        else:
            parent_bounds = stage3h_test_parent_bounds.get(relative)
            direct_parent = (
                parent_bounds is not None
                and parent_bounds[0] < declaration_offset < parent_bounds[1]
                and declaration_depth == 1
            )
        if not direct_parent:
            missing_stage3h_tests.append(f"{relative}::{name} (nested)")
            continue
        item = stage3b_function(relative, name, "stage3h-runtime-evidence")
        normalized_item = " ".join(item.split())
        item_hash = normalized_code_sha256(item)
        if (
            any(anchor not in normalized_item for anchor in anchors)
            or item_hash != stage3h_test_body_hashes.get((relative, name))
        ):
            drifted_stage3h_tests.append(f"{relative}::{name} ({item_hash})")
    if missing_stage3h_tests or drifted_stage3h_tests:
        fail(
            "stage3h-runtime-evidence",
            "Stage3H tests must remain unconditional direct-parent #[test] functions with exact counts/blocker vector, raw111 typed handoff, natural/manual wires, object identity, primitive boxing, defining-realm prototypes, nullish pending/catch, rollback/retry, and branch-index evidence; "
            f"missing {missing_stage3h_tests}, drifted {drifted_stage3h_tests}",
        )

    stage3i_test_contracts = (
        (
            "src/runtime/binary_object/function_translate/capability.rs",
            "registry_locks_the_current_physical_cohorts",
            (
                "(111, 1, 103, 29)",
                "assert_eq!(scalar_only + shared, 30);",
                "assert_eq!(ordinary_only + shared, 132);",
                "assert_eq!(scalar_only + ordinary_only + shared, 133);",
            ),
        ),
        (
            "src/runtime/binary_object/function_translate/capability.rs",
            "ordinary_push_this_is_the_exact_operand_free_raw8_row",
            (
                "CAPABILITY_REGISTRY[8]",
                "OpcodeFormat::None",
                "CapabilityPolicy::OrdinaryOnly(Recipe::PushThis)",
                "opcode.n_pop(), 0",
                "opcode.n_push(), 1",
                "CAPABILITY_REGISTRY[47].policy",
                "CapabilityPolicy::Blocked(TranslationBlocker::Completion)",
                "CAPABILITY_REGISTRY[112].policy",
                "CapabilityPolicy::Blocked(TranslationBlocker::ValueConstruction)",
            ),
        ),
        (
            "src/runtime/binary_object/function_translate/capability.rs",
            "blocked_frontier_has_stable_typed_category_counts",
            (
                "[1, 5, 2, 1, 3, 7, 16, 15, 25, 4, 9, 11, 5, 4, 3]",
                "counts.into_iter().sum::<usize>(), 111",
            ),
        ),
        (
            "src/runtime/binary_object/function_translate/mod.rs",
            "operand_free_push_this_translation_is_one_ordinary_typed_operation",
            (
                "lower_operation(Recipe::PushThis, &NativeOperands::None).unwrap()",
                "Some(PendingOperation::Ready(FunctionOp::PushThis))",
                "assert!(operations.next().is_none());",
                "TranslationTarget::Scalar",
                "Some(PendingOperation::Ready(FunctionOp::OutsideTarget))",
            ),
        ),
        (
            "src/runtime/binary_object/ordinary_leaf.rs",
            "lowers_representative_sanitized_operations_without_consulting_diagnostics",
            (("FunctionOp::PushThis, OrdinaryLeafOp::PushThis"),),
        ),
        (
            "src/runtime/binary_object/ordinary_leaf.rs",
            "push_this_wires_preserve_strictness_source_order_and_one_to_one_lowering",
            (
                "NATURAL_STRICT_PUSH_THIS_HEX",
                "NATURAL_SLOPPY_PUSH_THIS_HEX",
                "MINIMAL_STRICT_PUSH_THIS_HEX",
                "MINIMAL_SLOPPY_PUSH_THIS_HEX",
                "OrdinaryLeafOp::PushThis, OrdinaryLeafOp::PutLocal(0), OrdinaryLeafOp::GetLocal(0), OrdinaryLeafOp::Return",
                "vec![OrdinaryLeafOp::PushThis, OrdinaryLeafOp::Return]",
                "draft.metadata().max_stack(), 1",
                "draft.constants().is_empty()",
            ),
        ),
        (
            "src/runtime/binary_object/ordinary_leaf.rs",
            "push_this_protocol_rejects_duplicate_nonzero_and_branch_target_zero",
            (
                "duplicate.insert(40, 8);",
                "nonzero.insert(39, 177);",
                "branch_target_zero.extend_from_slice(&[8, 234, (-2_i8) as u8, 40]);",
                "OrdinaryLeafReadError::Unadmitted(message)",
            ),
        ),
        (
            "src/runtime/binary_object/ordinary_leaf.rs",
            "push_this_protocol_preserves_raw8_absent_branch_target_zero",
            (
                "no_push_this.extend_from_slice(&[177, 234, (-2_i8) as u8, 41]);",
                "OrdinaryLeafOp::Nop, OrdinaryLeafOp::Goto(0), OrdinaryLeafOp::ReturnUndefined",
            ),
        ),
        (
            "src/runtime/binary_object_publish.rs",
            "ordinary_leaf_draft_ops_lower_one_for_one_without_reordering",
            ("lower(OrdinaryLeafOp::PushThis)", "Instruction::PushThis"),
        ),
        (
            "src/runtime/binary_object_publish.rs",
            "ordinary_push_this_publishes_one_for_one_without_a_synthetic_constant",
            (
                "let mut next_synthetic_index = 7;",
                "lower_ordinary_leaf_op(OrdinaryLeafOp::PushThis, &mut next_synthetic_index)",
                "Ok(Instruction::PushThis)",
                "assert_eq!(next_synthetic_index, 7);",
            ),
        ),
        (
            "src/runtime/tests.rs",
            "push_this_applies_strict_and_sloppy_callee_realm_rules",
            (
                "Instruction::PushThis",
                "Value::Undefined",
                "Value::Object(global)",
                "Some(context.number_prototype().unwrap())",
            ),
        ),
        (
            "src/runtime/tests.rs",
            "trusted_quickjs_ordinary_push_this_is_exact_typed_and_realm_correct",
            (
                "QUICKJS_NATURAL_STRICT_PUSH_THIS_BC5",
                "0x4ec7_e018_7375_d810",
                "QUICKJS_ORDINARY_SLOPPY_PUSH_THIS_BC5",
                "0x0e24_85c9_7eea_9cfa",
                "Instruction::PushThis",
                "Instruction::PutLocal(0)",
                "Instruction::GetLocal(0)",
                "snapshot.constants.is_empty()",
                "snapshot.metadata.max_stack, 1",
                "Value::Object(defining_global.clone())",
                "assert_ne!( first, second",
                "Some(defining_prototype.clone())",
                "same_value(primitive)",
            ),
        ),
        (
            "src/runtime/tests.rs",
            "trusted_quickjs_ordinary_push_this_protocol_rejects_transactionally_and_retries",
            (
                "QUICKJS_DUPLICATE_PUSH_THIS_BC5.len(), 43",
                "QUICKJS_REENTER_PUSH_THIS_BC5.len(), 65",
                "nonzero.insert(39, 177);",
                "for (label, branch_encoding) in branch_encodings",
                "assert_eq!(runtime.heap_counts(), baseline",
                "assert_eq!(runtime.test_atom_count(), baseline_atoms",
                "read_trusted_ordinary_function(QUICKJS_ORDINARY_STRICT_PUSH_THIS_BC5, 0)",
                "Value::Int(42)",
            ),
        ),
    )
    stage3i_test_body_hashes = {
        ("src/runtime/binary_object/function_translate/capability.rs", "registry_locks_the_current_physical_cohorts"): "3ff8521f17f3acd991542c7de3e769507ba061eb19d605a413e4febe65f77cc9",
        ("src/runtime/binary_object/function_translate/capability.rs", "ordinary_push_this_is_the_exact_operand_free_raw8_row"): "3064e195a551749f3e10ede4d064b138798f8f378ea3ffc6b539ff961c4fbde2",
        ("src/runtime/binary_object/function_translate/capability.rs", "blocked_frontier_has_stable_typed_category_counts"): "601aa4392300e4a3f965cef80787945d4aea706fdc729325318a016e17bb41c8",
        ("src/runtime/binary_object/function_translate/mod.rs", "operand_free_push_this_translation_is_one_ordinary_typed_operation"): "47ce4ec9602857ae10e21b026cb06c07d3bda12253959a8201f31be71e0319ab",
        ("src/runtime/binary_object/ordinary_leaf.rs", "lowers_representative_sanitized_operations_without_consulting_diagnostics"): "0da974adaa4d87c8aa9949f1ab1ab764b6408aafcf426d3f3218d426965a697d",
        ("src/runtime/binary_object/ordinary_leaf.rs", "push_this_wires_preserve_strictness_source_order_and_one_to_one_lowering"): "b3f94bf497c8c4683396cd5df28b6ca2a3d77a126fe81759c38d976eeaff1952",
        ("src/runtime/binary_object/ordinary_leaf.rs", "push_this_protocol_rejects_duplicate_nonzero_and_branch_target_zero"): "0953065a02ef76dde92b29aa1b687ae3b535376bbb098ddcd45903b59fc64fdc",
        ("src/runtime/binary_object/ordinary_leaf.rs", "push_this_protocol_preserves_raw8_absent_branch_target_zero"): "15384f1727b7b95c64e3f2cfb5163795baae4a1089455073718d4aa518373919",
        ("src/runtime/binary_object_publish.rs", "ordinary_leaf_draft_ops_lower_one_for_one_without_reordering"): "d00312170558a488c86b0eb21eeea031f155e0ddb9087e5f3219619282f8706d",
        ("src/runtime/binary_object_publish.rs", "ordinary_push_this_publishes_one_for_one_without_a_synthetic_constant"): "87adcb251ea56538be8045fc59dd788b9f8ae3649a3a77aba7bfd052d3636497",
        ("src/runtime/tests.rs", "push_this_applies_strict_and_sloppy_callee_realm_rules"): "b546bfdc3c65105cdab2dbbc62cb44ecf827934b79903deeb4b3fbfc6da86113",
        ("src/runtime/tests.rs", "trusted_quickjs_ordinary_push_this_is_exact_typed_and_realm_correct"): "793e23d69dc7a6d2602ed19e41b1eef5c90a0179b5674a6c2d8a8e31211e28d1",
        ("src/runtime/tests.rs", "trusted_quickjs_ordinary_push_this_protocol_rejects_transactionally_and_retries"): "3290dcd11a72cd1b107d0ae552b603664ea8d01df72e06c9ef2acf5ae1b0a3e3",
    }
    stage3i_test_parent_bounds = dict(stage3d_test_parent_bounds)
    stage3i_test_sources = {relative for relative, _, _ in stage3i_test_contracts}
    for relative in stage3i_test_sources:
        code = stage3b_code(relative)
        if assertion_shadow.search(code):
            fail(
                "stage3i-runtime-evidence",
                f"{relative} must not shadow or import the assertion macros used by Stage3I evidence",
            )
        if re.search(r"(?m)^[ \t]*#![ \t]*\[[ \t]*(?:cfg|cfg_attr)\b", code):
            fail(
                "stage3i-runtime-evidence",
                f"{relative} must not conditionally exclude its Stage3I test evidence",
            )
    missing_stage3i_tests = []
    drifted_stage3i_tests = []
    for relative, name, anchors in stage3i_test_contracts:
        code = stage3b_code(relative)
        declarations = list(re.finditer(
            rf"(?P<attributes>(?:#[ \t\n]*\[[^]]*\][ \t\n]*)*)"
            rf"\bfn[ \t\n]+{re.escape(name)}[ \t\n]*\(",
            code,
        ))
        if (
            len(declarations) != 1
            or " ".join(declarations[0].group("attributes").split()) != "#[test]"
        ):
            missing_stage3i_tests.append(f"{relative}::{name}")
            continue
        declaration_offset = declarations[0].start()
        declaration_depth = code[:declaration_offset].count("{") - code[:declaration_offset].count("}")
        if relative == "src/runtime/tests.rs":
            direct_parent = declaration_depth == 0
        else:
            parent_bounds = stage3i_test_parent_bounds.get(relative)
            direct_parent = (
                parent_bounds is not None
                and parent_bounds[0] < declaration_offset < parent_bounds[1]
                and declaration_depth == 1
            )
        if not direct_parent:
            missing_stage3i_tests.append(f"{relative}::{name} (nested)")
            continue
        item = stage3b_function(relative, name, "stage3i-runtime-evidence")
        normalized_item = " ".join(item.split())
        item_hash = normalized_code_sha256(item)
        if (
            any(anchor not in normalized_item for anchor in anchors)
            or item_hash != stage3i_test_body_hashes.get((relative, name))
        ):
            drifted_stage3i_tests.append(f"{relative}::{name} ({item_hash})")
    if missing_stage3i_tests or drifted_stage3i_tests:
        fail(
            "stage3i-runtime-evidence",
            "Stage3I tests must remain unconditional direct-parent #[test] functions with exact counts/blockers, raw8 typed chain and source/output order, no synthetic constant, strict/sloppy wires and realm semantics, all entrance-protocol negatives, raw8-absent compatibility, transactional rollback, and retry evidence; "
            f"missing {missing_stage3i_tests}, drifted {drifted_stage3i_tests}",
        )

    stage3i_c_evidence_hashes = {
        "tests/fixtures/function_bytecode_wire.c": "e6d93033db5e00b403ab203e598bc66f77d079329680e970d779d63a388ff0c4",
        "tests/fixtures/function_bytecode_wire.quickjs-2026-06-04.txt": "5750753443089a599e2863dcad6d282c597b27cf1683b48e3ab76664091e71e6",
        "dev-support/quickjs-c-oracles.tsv": "7e90cdbc0c7570050eb983e7ddfdea32914ff9e753dd986a654ac9c56d7ea355",
    }
    stage3d_c_sources: dict[str, str] = {}
    for relative, expected_hash in stage3i_c_evidence_hashes.items():
        path = root / relative
        if path.is_symlink() or not path.is_file():
            fail("stage3i-c-oracle", f"{relative} must remain a regular authenticated file")
            continue
        payload = path.read_bytes()
        found_hash = hashlib.sha256(payload).hexdigest()
        if found_hash != expected_hash:
            fail(
                "stage3i-c-oracle",
                f"{relative} drifted from frozen C3 diff 70f9b90d8e11484bdde52e68bae70bf0937f36a0b036442bbe412dad60cf1fdd; found {found_hash}",
            )
        stage3d_c_sources[relative] = payload.decode("utf-8")

    stage3d_c_source = stage3d_c_sources.get(
        "tests/fixtures/function_bytecode_wire.c", ""
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
        fail(
            "stage3d-c-oracle",
            "the authenticated C oracle must define and call exactly one raw48, raw49, raw177, compiler-natural raw11 Object, and raw111 ToObject case, with distinct honest natural/manual wires and exact mechanical derivations",
        )
    if (
        len(stage3d_c_source.splitlines()) != 8953
        or stage3d_c_source.count(
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
        fail(
            "stage3i-c-oracle",
            "the exact 8,953-line C oracle must define and call one Stage3I raw8 matrix, retain all six exact natural/manual/adversarial wires, and prove duplicate and branch-to-index-zero re-execution",
        )
    stage3d_c_transcript = stage3d_c_sources.get(
        "tests/fixtures/function_bytecode_wire.quickjs-2026-06-04.txt", ""
    )
    stage3d_c_transcript_contract = (
        "ordinary-throw-wire-size=45",
        "ordinary-throw-wire-fnv1a64=73cf217e06c5fee2",
        "ordinary-throw-wire-sha256=b7998b9678635e7e0a4eb2e465b683d168395adc7f156f733c25521907e3c8a8",
        "ordinary-throw-child-metadata=flags:0243,js_mode:1,args:1,vars:0,defined_args:1,stack:1,var_refs:0,closures:0,cpool:0,code:2,locals:1,code_offset:43",
        "ordinary-throw-child-code-hex=cf30",
        "ordinary-throw-child-code-raw=207,48",
        "ordinary-throw-terminal=raw48,stack:1->0,no-return",
        "ordinary-throw-caller-catch=int,object,Error:original-identity;terminal-no-return",
        "ordinary-throw-iterator-close=body,return,catch-original;close-throw-does-not-replace-original",
        "ordinary-throw-oracle=passed",
    )
    if any(
        stage3d_c_transcript.count(f"{line}\n") != 1
        for line in stage3d_c_transcript_contract
    ):
        fail(
            "stage3d-c-oracle",
            "the frozen C transcript must retain the exact raw48 wire, metadata, terminal, identity, backtrace, and iterator-close evidence",
        )
    stage3e_c_transcript_contract = (
        "ordinary-throw-error-natural-wire-size=58",
        "ordinary-throw-error-natural-wire-fnv1a64=026914eda60a481f",
        "ordinary-throw-error-natural-wire-sha256=a07b3f39a5e3929af4899a07686e91324e4ee9c54b729f518813eaa4a1875199",
        "ordinary-throw-error-natural-child-code-hex=5e0000b3c7b41131f300000000",
        "ordinary-throw-error-natural-terminal=raw49/subtype0,stack:0->0,no-return",
        "ordinary-throw-error-natural-ordinary-cohort-exclusion=lexical-vars:1,locals:1,local-flags:b0,raw94:set_loc_uninitialized",
        "ordinary-throw-error-wire-size=47",
        "ordinary-throw-error-wire-fnv1a64=b4c1126c283093af",
        "ordinary-throw-error-wire-sha256=d05cabd4c18598b024f66eab8fd723c412fc5a469325b26fca5042507dea3ee8",
        "ordinary-throw-error-child-metadata=flags:0243,js_mode:1,args:0,vars:0,defined_args:0,stack:0,var_refs:0,closures:0,cpool:0,code:6,locals:0,code_offset:41",
        "ordinary-throw-error-child-code-hex=31f300000000",
        "ordinary-throw-error-child-raw=49",
        "ordinary-throw-error-terminal=raw49/subtype0,stack:0->0,no-return",
        "ordinary-throw-error-empty-stack=metadata-max-stack:0;raw49:0->0;TypeError-not-underflow",
        "ordinary-throw-error-unicode-wire=atom:x->U+00E9;size:47;rewrite:identity;fnv1a64:b733634a7dff678e;sha256:8228fdf15ff5551e6e14bac89e91d606c2aba6fe5d7ded834c309830842fd324",
        "ordinary-throw-error-realm=defining-TypeError:true;caller-TypeError:false",
        "ordinary-throw-error-backtrace=own-stack-before-catch;anonymous-frame-present",
        "ordinary-throw-error-pending=direct-call-publishes;GetException-clears;caller-catch-clears",
        "ordinary-throw-error-caller-catch=Unicode-TypeError:defining-realm;terminal-no-return;result:42",
        "ordinary-throw-error-subtype1=fresh-read-write-exec:SyntaxError:redeclaration-of-x;Rust:Unadmitted",
        "ordinary-throw-error-subtype255=fresh-read-write-exec:InternalError:invalid-throw-var-type-255;Rust:Unadmitted",
        "ordinary-throw-error-rust-admission=raw49/subtype0-only;subtype1-255:Unadmitted",
        "ordinary-exception-admitted-raw=48,49",
        "ordinary-throw-error-oracle=passed",
    )
    if any(
        stage3d_c_transcript.count(f"{line}\n") != 1
        for line in stage3e_c_transcript_contract
    ):
        fail(
            "stage3e-c-oracle",
            "the frozen C transcript must retain both raw49 wire identities, subtype-0 terminal semantics, realm/backtrace/pending/catch evidence, and subtype rejection contrast",
        )

    stage3f_c_transcript_contract = (
        "ordinary-nop-evidence=compiler-natural-raw41-baseline-plus-mechanical-raw177-insertion",
        "ordinary-nop-natural-wire-size=40",
        "ordinary-nop-natural-wire-fnv1a64=bb77ba50387051a2",
        "ordinary-nop-natural-wire-sha256=a50422c2b092ab4162505321642241e7d24c43c5617e4b4ef0d076cde44b6f92",
        "ordinary-nop-natural-child-metadata=flags:0243,js_mode:1,args:0,vars:0,defined_args:0,stack:0,var_refs:0,closures:0,cpool:0,code:1,locals:0,code_offset:39,atoms:0",
        "ordinary-nop-natural-child-code-hex=29",
        "ordinary-nop-natural-child-raw=41",
        "ordinary-nop-natural-provenance=raw41-only;raw177-absent-never-compiler-natural",
        "ordinary-nop-wire-size=41",
        "ordinary-nop-wire-fnv1a64=1c522736e3cbef92",
        "ordinary-nop-wire-sha256=26c2e58ec14861dc797a7c3a3701f258ba392b649a15554256b61d7634fccdd0",
        "ordinary-nop-child-metadata=flags:0243,js_mode:1,args:0,vars:0,defined_args:0,stack:0,var_refs:0,closures:0,cpool:0,code:2,locals:0,code_offset:39,atoms:0",
        "ordinary-nop-child-code-hex=b129",
        "ordinary-nop-child-raw=41,177",
        "ordinary-nop-derivation=natural40:code1->2;insert-raw177-before-natural-raw41",
        "ordinary-nop-property-free=atoms:0,args:0,vars:0,var_refs:0,closures:0,cpool:0,locals:0,stack:0",
        "ordinary-nop-rewrite=identity,fresh-root:Function",
        "ordinary-nop-call=defining-realm-twice:undefined;caller-realm-twice:undefined",
        "ordinary-nop-pending=none-before-or-after-repeat-cross-realm",
        "ordinary-nop-only-boundary=40-byte-raw177-only:Rust-verifier-negative-only;C-never-reads-or-executes",
        "ordinary-nop-admitted-raw=177",
        "ordinary-nop-oracle=passed",
    )
    if any(
        stage3d_c_transcript.count(f"{line}\n") != 1
        for line in stage3f_c_transcript_contract
    ):
        fail(
            "stage3f-c-oracle",
            "the frozen C transcript must retain the exact compiler-natural raw41 baseline, mechanically derived raw177/raw41 wire, zero-property metadata, byte identity, repeated cross-realm undefined calls, empty pending state, and C-never-executes malformed boundary",
        )

    stage3g_c_transcript_contract = (
        "ordinary-object-evidence=compiler-natural-write-read-write-fresh-runtime",
        "ordinary-object-compile-mode=global-compile-only,strip-debug",
        "ordinary-object-wire-size=41",
        "ordinary-object-wire-fnv1a64=3c41af3fef8b3a1e",
        "ordinary-object-wire-sha256=a58ccbed5658ba6a9de99e909d5ba0b4af59ad47fccf0f5cccdff072d6494db9",
        "ordinary-object-wire-hex=05000c000200a80100010001000001040100000000be00cb280c430201000000000100000002000b28",
        "ordinary-object-child-metadata=flags:0243,js_mode:1,args:0,vars:0,defined_args:0,stack:1,var_refs:0,closures:0,cpool:0,code:2,locals:0,code_offset:39,atoms:0",
        "ordinary-object-child-code-hex=0b28",
        "ordinary-object-child-raw=11,40",
        "ordinary-object-terminal=raw40;raw11:0->1,raw40:1->0",
        "ordinary-object-property-free=atoms:0,args:0,vars:0,var_refs:0,closures:0,cpool:0,locals:0,stack:1",
        "ordinary-object-rewrite=identity,fresh-root:Function",
        "ordinary-object-call=defining-realm-twice:distinct;caller-realm-twice:distinct;all-four:distinct",
        "ordinary-object-prototype=all:defining-realm-Object.prototype;caller-realm:false",
        "ordinary-object-extensible=all:true",
        "ordinary-object-own-properties=all:0",
        "ordinary-object-pending=none-before-or-after-repeat-cross-realm",
        "ordinary-object-admitted-count=1",
        "ordinary-object-admitted-raw=11",
        "ordinary-object-oracle=passed",
    )
    if any(
        stage3d_c_transcript.count(f"{line}\n") != 1
        for line in stage3g_c_transcript_contract
    ):
        fail(
            "stage3g-c-oracle",
            "the frozen C transcript must retain the exact compiler-natural raw11 Object wire, property-free max-stack-one metadata, read/write identity, fresh defining-realm Objects, and clean pending state",
        )

    stage3h_c_transcript_contract = (
        "ordinary-to-object-evidence=compiler-natural-provenance-plus-mechanically-derived-property-free-wire",
        "ordinary-to-object-natural-source-hex=2866756e6374696f6e2861297b2775736520737472696374273b287b7d3d61293b72657475726e20613b7d29",
        "ordinary-to-object-natural-compile-mode=global-compile-only,strip-debug",
        "ordinary-to-object-natural-wire-size=56",
        "ordinary-to-object-natural-wire-fnv1a64=65a8b3d0d7ed115a",
        "ordinary-to-object-natural-wire-sha256=f5bdac14901bb6b752e2ca10a01dd31d6990456c43f78d5923b1da4a0ef3706e",
        "ordinary-to-object-natural-wire-hex=05000c000200a80100010001000001040100000000be00cb280c43020100010001020000000d0100010000ea06116f0eea04cfeaf90ecf28",
        "ordinary-to-object-natural-child-metadata=flags:0243,js_mode:1,args:1,vars:0,defined_args:1,stack:2,var_refs:0,closures:0,cpool:0,code:13,locals:1,code_offset:43,atoms:0",
        "ordinary-to-object-natural-child-code-hex=ea06116f0eea04cfeaf90ecf28",
        "ordinary-to-object-natural-child-raw=14,17,40,111,207,234",
        "ordinary-to-object-natural-terminal=raw40;raw111:1->1,raw40:1->0",
        "ordinary-to-object-natural-provenance=compiler-emitted-empty-object-destructuring;returns-original-argument",
        "ordinary-to-object-wire-size=46",
        "ordinary-to-object-wire-fnv1a64=c84f87720cd09b16",
        "ordinary-to-object-wire-sha256=13f81e66520578393a57f3290636d4778c5cae8d014591e5daaaacdd3ffd5c95",
        "ordinary-to-object-wire-hex=05000c000200a80100010001000001040100000000be00cb280c4302010001000101000000030100010000cf6f28",
        "ordinary-to-object-child-metadata=flags:0243,js_mode:1,args:1,vars:0,defined_args:1,stack:1,var_refs:0,closures:0,cpool:0,code:3,locals:1,code_offset:43,atoms:0",
        "ordinary-to-object-child-code-hex=cf6f28",
        "ordinary-to-object-child-raw=40,111,207",
        "ordinary-to-object-terminal=raw40;raw111:1->1,raw40:1->0",
        "ordinary-to-object-derivation=natural56:stack2->1,code13->3;replace-control-flow-dup-drop-with-cf6f28",
        "ordinary-to-object-property-free=atoms:0,args:1,vars:0,var_refs:0,closures:0,cpool:0,locals:1,stack:1",
        "ordinary-to-object-rewrite=identity,fresh-root:Function",
        "ordinary-to-object-object=caller-realm-input:original-identity;no-wrapper",
        "ordinary-to-object-primitives=Boolean,integer-Number,floating-Number,String,BigInt,Symbol",
        "ordinary-to-object-wrappers=each-call:fresh;payload:valueOf-original",
        "ordinary-to-object-prototype=all:defining-realm-intrinsic;caller-realm:false",
        "ordinary-to-object-user-coercion=valueOf,toString,Symbol.toPrimitive:0-calls",
        "ordinary-to-object-nullish=null,undefined:TypeError:cannot-convert-to-object",
        "ordinary-to-object-pending=direct-call-publishes;GetException-clears;caller-catch-clears",
        "ordinary-to-object-caller-catch=null,undefined:defining-TypeError:true;caller-TypeError:false;result:42",
        "ordinary-to-object-admitted-count=1",
        "ordinary-to-object-admitted-raw=111",
        "ordinary-to-object-oracle=passed",
    )
    if any(
        stage3d_c_transcript.count(f"{line}\n") != 1
        for line in stage3h_c_transcript_contract
    ):
        fail(
            "stage3h-c-oracle",
            "the frozen C transcript must retain the natural and mechanical raw111 wire identities, metadata/raw sets, one-to-one stack semantics, object identity, defining-realm primitive boxing, no-coercion, nullish pending/GetException clearing, and caller-catch evidence",
        )

    stage3i_c_transcript_contract = (
        "ordinary-push-this-evidence=compiler-natural-strict-and-sloppy-plus-mechanically-derived-property-free-wires",
        "ordinary-push-this-strict-natural-wire-size=47",
        "ordinary-push-this-strict-natural-wire-fnv1a64=4ec7e0187375d810",
        "ordinary-push-this-strict-natural-wire-sha256=786376192d5bfe7eb07115f62788707619ee54e8721acfa66dae1d110a580e39",
        "ordinary-push-this-strict-natural-child-code-hex=08c7c328",
        "ordinary-push-this-strict-natural-provenance=compiler-emitted-return-this;js_mode:strict;raw8-exact-one",
        "ordinary-push-this-strict-wire-size=41",
        "ordinary-push-this-strict-wire-fnv1a64=3c3e393fef883bc5",
        "ordinary-push-this-strict-wire-sha256=9b14c5245a78e0a069967089cf6f89aefac3e12749d16eba36e4c15b72a3c99e",
        "ordinary-push-this-strict-child-code-hex=0828",
        "ordinary-push-this-sloppy-natural-wire-size=47",
        "ordinary-push-this-sloppy-natural-wire-fnv1a64=4e7f8f98adff8463",
        "ordinary-push-this-sloppy-natural-wire-sha256=f0430a7c241caaf94703bd5de73289d4f90fea3ee9cfaf22a660ed80df3de0a6",
        "ordinary-push-this-sloppy-natural-child-code-hex=08c7c328",
        "ordinary-push-this-sloppy-natural-provenance=compiler-emitted-return-this;js_mode:sloppy;raw8-exact-one",
        "ordinary-push-this-sloppy-wire-size=41",
        "ordinary-push-this-sloppy-wire-fnv1a64=0e2485c97eea9cfa",
        "ordinary-push-this-sloppy-wire-sha256=213b3b6a332d4cf69e4c726b372c1f0087e70fc9c263a6a2193ce4763fb62648",
        "ordinary-push-this-sloppy-child-code-hex=0828",
        "ordinary-push-this-duplicate-wire-size=43",
        "ordinary-push-this-duplicate-wire-fnv1a64=920de09aaf63833e",
        "ordinary-push-this-duplicate-wire-sha256=9f0541bfd8a599e5f2575936d24df9a2487a1e8952fca1648afeef5c9f798a30",
        "ordinary-push-this-duplicate-raw8-occurrences=2",
        "ordinary-push-this-duplicate-execution=sloppy-primitive-this:1;two-raw8-wrappers-strict-eq:false",
        "ordinary-push-this-loop-wire-size=65",
        "ordinary-push-this-loop-wire-fnv1a64=fa100ff2b0854673",
        "ordinary-push-this-loop-wire-sha256=32b4c9e45f5191d21aa44d3437c54b00cfa1ff4b2530d1e4cdf942a87e8f3fb4",
        "ordinary-push-this-loop-raw8-occurrences=1",
        "ordinary-push-this-loop-branch-map=raw105-operand@3:+12->15;raw106-operand@11:-11->0",
        "ordinary-push-this-loop-execution=sloppy-primitive-this:1;single-raw8-executed-twice;wrappers-strict-eq:false",
        "ordinary-push-this-mode-delta=strict-vs-sloppy:js_mode-byte28-only;natural-and-manual",
        "ordinary-push-this-read-write=strict-natural,sloppy-natural,strict-manual,sloppy-manual:identity",
        "ordinary-push-this-strict=undefined:undefined;null:null;primitives:exact-no-boxing;object:identity",
        "ordinary-push-this-sloppy-nullish=undefined,null:defining-global;caller-global:false",
        "ordinary-push-this-sloppy-wrappers=natural-and-manual:each-call-fresh;two-calls-per-primitive:distinct",
        "ordinary-push-this-sloppy-prototype=all-repeat-wrappers:defining-realm-intrinsic;caller-realm:false",
        "ordinary-push-this-sloppy-payload=all-repeat-wrappers:valueOf-original",
        "ordinary-push-this-admission-boundary=require-exact-one-raw8-and-no-branch-target0;duplicate-and-loop-prove-each-raw8-execution-reboxes",
        "ordinary-push-this-pending=none-before-or-after-strict-sloppy-repeat-cross-realm-or-adversarial",
        "ordinary-push-this-admitted-count=1",
        "ordinary-push-this-admitted-raw=8",
        "ordinary-push-this-oracle=passed",
    )
    if (
        stage3d_c_transcript.count("\n") != 1594
        or any(
            stage3d_c_transcript.count(f"{line}\n") != 1
            for line in stage3i_c_transcript_contract
        )
    ):
        fail(
            "stage3i-c-oracle",
            "the exact 1,594-line C transcript must retain all strict/sloppy natural/manual raw8 wire identities, metadata and provenance, fresh defining-realm boxing, exact payloads, duplicate and target-zero mismatch probes, pending state, and sole-raw8 admission evidence",
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
                "tests/fixtures/function_bytecode_wire.c\t"
                "e6d93033db5e00b403ab203e598bc66f77d079329680e970d779d63a388ff0c4\t"
                "tests/fixtures/function_bytecode_wire.quickjs-2026-06-04.txt\t"
                "5750753443089a599e2863dcad6d282c597b27cf1683b48e3ab76664091e71e6\t"
            )
            for line in stage3i_manifest_lines
        )
        != 1
    ):
        fail(
            "stage3i-c-oracle",
            "the exact 20-line authenticated manifest must retain 19 fixtures and one function-bytecode-wire row pinning the frozen Stage3I C source and transcript hashes",
        )

    stage3e_status = read_source("docs/status.md")
    for description, start_fragment, end_fragment, expected_hash in (
        (
            "the status document must retain the exact explicit-throw typed path and no-VM-change boundary",
            "Stage 3D admits explicit raw 48",
            "already implemented exception path.",
            "9f1793c230bff05e3d1c8ea6e8e80b9dbe4ec64815a7fe684ffe4e58959e0f22",
        ),
        (
            "the status document must retain the exact Rust raw48 identity, backtrace, iterator-close, terminal, and rollback evidence",
            "Stage-3D Rust evidence uses",
            "transactional heap/atom rollback.",
            "8342c0f2e2b880cc8f0c668680db1521ca400a8cae2d837a261ed785fe6d3c09",
        ),
        (
            "the status document must retain the authenticated raw48 through raw8 C wires and current Stage3I source/transcript/manifest hashes",
            "Stage 3D adds the exact compiler-natural strict 45-byte raw-48 wire",
            "`7e90cdbc0c7570050eb983e7ddfdea32914ff9e753dd986a654ac9c56d7ea355`.",
            "97e94a3cae6358e553b2f3bebc596554c210eff69e1d034ad914c493db155e7a",
        ),
        (
            "the status document must retain the exact Stage3E typed atom/synthetic-constant path and no-new-VM boundary",
            "Stage 3E admits raw 49 only as the typed chain",
            "public API, source syntax, Test262 admission, or Feature Parity claim.",
            "f9772990f611da10923dded7873f94c581ae82c6c3cc5517daeb76e9b0fba341",
        ),
        (
            "the status document must retain the exact Stage3E natural/manual wire, atom provenance, terminal, realm, pending, catch, and rollback evidence",
            "Stage-3E Rust evidence distinguishes",
            "transactional retry after every rejected form.",
            "b52a229d39453fd9ef6abee7039e873cf14c9c42266e82c883850d7c612761c4",
        ),
    ):
        require_normalized_corridor_sha256(
            "stage3e-status",
            description,
            stage3e_status,
            start_fragment,
            end_fragment,
            expected_hash,
        )

    stage3f_status_corridors = (
        (
            "the status document must retain the exact Stage3F raw177 one-to-one Nop chain, branch index, verifier fallthrough, and VM no-effect boundary",
            "Stage 3F admits raw 177 only as the exact one-to-one typed chain",
            "adds no public surface, source syntax, Test262 admission, or Feature Parity claim.",
            "068db26e6d2a60b8336443ace78bc6bf165b37733e1bce10282abbcb6b646671",
        ),
        (
            "the status document must retain the exact Stage3F 41-byte wire, metadata, realm, pending, malformed-fallthrough rollback, and branch-index evidence",
            "Stage-3F Rust evidence uses the exact 41-byte property-free strict function wire",
            "a separate branch fixture proves `Goto(1)` still lands on `Instruction::Nop`.",
            "d4af12e9a2e3b169ad8f8d2061647630d71eaa473a80862690df896f3e9cb0f9",
        ),
        (
            "the status document must retain the exact Stage3F compiler-natural baseline, mechanical raw177 derivation, Stage3G Object, Stage3H ToObject, and Stage3I PushThis evidence, C execution boundary, and frozen C hashes",
            "Stage 3F authenticates the compiler-natural strict empty-function baseline",
            "`7e90cdbc0c7570050eb983e7ddfdea32914ff9e753dd986a654ac9c56d7ea355`.",
            "694e00e860bcdfeddc23807604b871dc35acadb97c42dc6f25e907d9fd096b62",
        ),
    )

    stage3g_status_corridors = (
        (
            "the status document must retain the exact Stage3G raw11 one-to-one Object chain, publisher, verifier, VM, defining-realm, raw47, and no-new-surface boundaries",
            "Stage 3G admits raw 11 only as the exact one-to-one typed chain",
            "Stage 3G exposes no new source syntax, public API, Test262 admission, or Feature Parity claim.",
            "c2cd9c362a26adfbf9854a9d1bd924ecf6a732391c51e879c02b200b81c4fbf3",
        ),
        (
            "the status document must retain the exact Stage3G 41-byte natural wire, metadata, fresh defining-realm objects, transaction rollback/retry, and branch-index evidence",
            "Stage-3G Rust evidence uses the compiler-natural exact 41-byte strict object-return wire",
            "typed raw11 index, fresh identity, defining prototype, and clean pending state.",
            "467a973c533455fd3956a967cae843ccb04e1817e01dd4468afe64a60f7ecec3",
        ),
    )

    stage3h_status_corridors = (
        (
            "the status document must retain the exact Stage3H raw111 one-to-one ToObject chain, verifier, VM, defining-realm boxing, no-coercion, blocked-neighbor, and no-new-surface boundaries",
            "Stage 3H admits raw 111 only as the exact one-to-one typed chain",
            "Stage 3H changes neither production bytecode nor VM implementation and adds no source syntax, public API, Test262 admission, or Feature Parity claim.",
            "40c7eb0c3221b0e01bac7000d884d57080c65ba83c248e2e685d0abc6877373e",
        ),
        (
            "the status document must retain the exact Stage3H natural/manual wires, metadata/raw sets, identity/boxing/realm/nullish semantics, rollback/retry, and branch-index evidence",
            "Stage-3H Rust evidence distinguishes the compiler-natural exact 56-byte strict",
            "Boolean boxing prototype, and clean pending state.",
            "04ff9df60b6dab1e8bbab1ffb4b5e1463a71e5963ccf3392a3d27528c1c0ebe6",
        ),
        (
            "the status document must retain honest compiler-natural versus mechanical C provenance, exact raw111 and raw8 execution evidence, and current frozen C hashes",
            "Stage 3H then compiler-naturally emits the exact 56-byte strict source",
            "`7e90cdbc0c7570050eb983e7ddfdea32914ff9e753dd986a654ac9c56d7ea355`.",
            "248a04f3f3d3f4570ab061ca789709c26f87b4498815aa4f91fc2c2e421d6d08",
        ),
    )

    stage3i_status_corridors = (
        (
            "the status document must retain the exact Stage3I scalar/ordinary/union cohorts, admitted milestones, blocked raw47/raw112 frontier, and blocker vector",
            "The scalar policy remains 30 opcodes; the stage-3I ordinary policy is 132,",
            "`1, 5, 2, 1, 3, 7, 16, 15, 25, 4, 9, 11, 5, 4, 3`.",
            "6c3d9eea430b23d069502fb866ced30a41d52a2203d38137b2ee7adca2977ff5",
        ),
        (
            "the status document must retain the exact Stage3I raw8 PushThis typed chain, archive protocol, VM receiver semantics, blocked neighbors, and no-new-surface boundary",
            "Stage 3I admits raw 8 only as the exact one-to-one typed chain",
            "Stage 3I changes neither the engine Instruction set nor VM implementation and adds no source syntax, public API, Test262 admission, or Feature Parity claim.",
            "004e01121a61cb598b558a77f02861a4d5dd3a77b0be8349d5a803323ac4d8e2",
        ),
        (
            "the status document must retain the exact Stage3I natural/manual wires, typed output, receiver/realm/boxing semantics, protocol negatives, rollback/retry, and raw8-absent compatibility evidence",
            "Stage-3I Rust evidence pins compiler-natural strict and sloppy 47-byte",
            "protocol does not narrow older ordinary bodies.",
            "b2dc822ee1a0cdb05fb9dd87d06773d5d5a6f5326468a0d64c370fdab8822dc5",
        ),
        (
            "the status document must retain honest Stage3I compiler-natural versus mechanical C provenance, adversarial raw8 execution evidence, and current frozen C hashes",
            "Stage 3I additionally compiler-naturally emits strict and sloppy",
            "`7e90cdbc0c7570050eb983e7ddfdea32914ff9e753dd986a654ac9c56d7ea355`.",
            "437860358c239012ed4ce59a2f6322650e6b77174db0089a004359f8200ad623",
        ),
        (
            "the status document must retain the exact Stage3I source-current receipt, inherited Stage3H/Stage3G/Stage3F coverage, and no-new-conformance lifecycle boundary",
            "This promoted receipt is source-current for Stage 3I",
            "raw-177 coverage, and makes no new conformance claim.",
            "83659ec1f3f12a5db78ca0595f43b2da887bef0d0200cdab16c7523998e67e6a",
        ),
    )

    stage3f_void_html_tags = {
        "area", "base", "br", "col", "embed", "hr", "img", "input",
        "link", "meta", "param", "source", "track", "wbr",
    }

    def stage3f_raw_html_ancestor_stack_at(
        source: str,
        stop: int,
    ) -> tuple[str, ...]:
        stack: list[str] = []
        index = 0
        while index < stop:
            opening = source.find("<", index, stop)
            if opening < 0:
                break
            if source.startswith("<!--", opening):
                comment_end = source.find("-->", opening + 4)
                if comment_end < 0 or comment_end + 3 > stop:
                    stack.append("!--")
                    break
                index = comment_end + 3
                continue

            cursor = opening + 1
            closing = cursor < stop and source[cursor] == "/"
            if closing:
                cursor += 1
            while cursor < stop and source[cursor].isspace():
                cursor += 1
            tag_match = re.match(r"[A-Za-z][A-Za-z0-9:-]*", source[cursor:stop])
            if tag_match is None:
                index = opening + 1
                continue
            tag = tag_match.group(0).lower()
            cursor += len(tag_match.group(0))
            quote: str | None = None
            tag_end = -1
            while cursor < len(source):
                character = source[cursor]
                if quote is not None:
                    if character == quote:
                        quote = None
                elif character in {"'", '"'}:
                    quote = character
                elif character == ">":
                    tag_end = cursor
                    break
                cursor += 1
            if tag_end < 0 or tag_end >= stop:
                stack.append("!incomplete-tag")
                break

            tag_tail = source[opening + 1:tag_end]
            if closing:
                if stack and stack[-1] == tag:
                    stack.pop()
                else:
                    stack.append(f"!mispaired:/{tag}")
            elif (
                tag not in stage3f_void_html_tags
                and not tag_tail.rstrip().endswith("/")
            ):
                stack.append(tag)
            index = tag_end + 1
        return tuple(stack)

    def stage3f_active_fence_at(source: str, stop: int) -> tuple[str, int] | None:
        active: tuple[str, int] | None = None
        for line in source[:stop].splitlines():
            fence = re.match(
                r"^[ ]{0,3}(?P<marker>`{3,}|~{3,})(?P<tail>.*)$",
                line,
            )
            if fence is None:
                continue
            marker = fence.group("marker")
            tail = fence.group("tail")
            if active is None:
                if marker[0] != "`" or "`" not in tail:
                    active = (marker[0], len(marker))
            elif (
                marker[0] == active[0]
                and len(marker) >= active[1]
                and not tail.strip()
            ):
                active = None
        return active

    def require_stage3f_top_level_corridor(
        diagnostic: str,
        description: str,
        start_fragment: str,
        end_fragment: str,
    ) -> None:
        normalized_characters: list[str] = []
        normalized_offsets: list[int] = []
        for token in re.finditer(r"\S+", stage3e_status):
            if normalized_characters:
                normalized_characters.append(" ")
                normalized_offsets.append(token.start())
            for offset in range(token.start(), token.end()):
                normalized_characters.append(stage3e_status[offset])
                normalized_offsets.append(offset)
        normalized_source = "".join(normalized_characters)
        if (
            normalized_source.count(start_fragment) != 1
            or normalized_source.count(end_fragment) != 1
        ):
            fail(diagnostic, description)
            return
        normalized_start = normalized_source.find(start_fragment)
        normalized_end = (
            normalized_source.find(end_fragment, normalized_start)
            + len(end_fragment)
        )
        start = normalized_offsets[normalized_start]
        end = normalized_offsets[normalized_end - 1] + 1
        line_start = stage3e_status.rfind("\n", 0, start) + 1
        line_end = stage3e_status.find("\n", end)
        if line_end < 0:
            line_end = len(stage3e_status)
        corridor_lines = stage3e_status[line_start:line_end].splitlines()
        indented = any(
            line and re.match(r"(?:[ ]{4,}|\t)", line)
            for line in corridor_lines
        )
        if (
            indented
            or stage3f_active_fence_at(stage3e_status, start) is not None
            or stage3f_active_fence_at(stage3e_status, end) is not None
            or stage3f_raw_html_ancestor_stack_at(stage3e_status, start)
            or stage3f_raw_html_ancestor_stack_at(stage3e_status, end)
        ):
            fail(
                diagnostic,
                f"{description}; the complete corridor must remain top-level rendered Markdown at both boundaries, not an indented/fenced block, comment, or raw-HTML descendant",
            )

    for description, start_fragment, end_fragment, expected_hash in stage3f_status_corridors:
        require_normalized_corridor_sha256(
            "stage3f-status",
            description,
            stage3e_status,
            start_fragment,
            end_fragment,
            expected_hash,
        )
        require_stage3f_top_level_corridor(
            "stage3f-status",
            description,
            start_fragment,
            end_fragment,
        )

    for description, start_fragment, end_fragment, expected_hash in stage3g_status_corridors:
        require_normalized_corridor_sha256(
            "stage3g-status",
            description,
            stage3e_status,
            start_fragment,
            end_fragment,
            expected_hash,
        )
        require_stage3f_top_level_corridor(
            "stage3g-status",
            description,
            start_fragment,
            end_fragment,
        )

    for description, start_fragment, end_fragment, expected_hash in stage3h_status_corridors:
        require_normalized_corridor_sha256(
            "stage3h-status",
            description,
            stage3e_status,
            start_fragment,
            end_fragment,
            expected_hash,
        )
        require_stage3f_top_level_corridor(
            "stage3h-status",
            description,
            start_fragment,
            end_fragment,
        )

    for description, start_fragment, end_fragment, expected_hash in stage3i_status_corridors:
        require_normalized_corridor_sha256(
            "stage3i-status",
            description,
            stage3e_status,
            start_fragment,
            end_fragment,
            expected_hash,
        )
        require_stage3f_top_level_corridor(
            "stage3i-status",
            description,
            start_fragment,
            end_fragment,
        )

    stage3f_inline_stage_label_emphasis = re.compile(
        r"(?P<prefix>\bStage[- ]*3)"
        r"(?P<delimiter>\*{1,3}|_{1,3})"
        r"(?P<label>[HIJ])(?P=delimiter)(?![\w*_])",
        re.IGNORECASE,
    )

    def stage3f_strip_inline_stage_label_emphasis(source: str) -> str:
        return stage3f_inline_stage_label_emphasis.sub(
            lambda match: match.group("prefix") + match.group("label"),
            source,
        )

    for delimiter in ("*", "**", "***", "_", "__", "___"):
        for label in ("H", "I", "J"):
            emphasized = f"Stage 3{delimiter}{label}{delimiter} lifecycle"
            expected = f"Stage 3{label} lifecycle"
            if stage3f_strip_inline_stage_label_emphasis(emphasized) != expected:
                fail(
                    f"stage3{label.lower()}-status",
                    "the bounded inline Stage 3H/3I/3J emphasis normalization must strip one to three matching Markdown emphasis markers",
                )
    for negative_control in (
        "Stage 30*H* lifecycle",
        "Stage 3*K* lifecycle",
        "Stage 3****H**** lifecycle",
        r"Stage 3\*H\* lifecycle",
        "Stage 3*H*idden lifecycle",
    ):
        if stage3f_strip_inline_stage_label_emphasis(negative_control) != negative_control:
            fail(
                "stage3h-status",
                "the bounded inline Stage 3H/3I/3J emphasis normalization must not consume non-label or escaped marker text",
            )

    def stage3f_rendered_text_projection(source: str) -> str:
        rendered: list[str] = []
        index = 0
        while index < len(source):
            if source.startswith("<!--", index):
                comment_end = source.find("-->", index + 4)
                index = len(source) if comment_end < 0 else comment_end + 3
                continue
            if source[index] == "<":
                cursor = index + 1
                if cursor < len(source) and source[cursor] == "/":
                    cursor += 1
                while cursor < len(source) and source[cursor].isspace():
                    cursor += 1
                tag_match = re.match(
                    r"[A-Za-z][A-Za-z0-9:-]*",
                    source[cursor:],
                )
                if tag_match is not None:
                    cursor += len(tag_match.group(0))
                    quote: str | None = None
                    while cursor < len(source):
                        character = source[cursor]
                        if quote is not None:
                            if character == quote:
                                quote = None
                        elif character in {"'", '"'}:
                            quote = character
                        elif character == ">":
                            index = cursor + 1
                            break
                        cursor += 1
                    else:
                        index = len(source)
                    continue
            rendered.append(source[index])
            index += 1

        projected = " ".join(html.unescape("".join(rendered)).split())
        projected = stage3f_strip_inline_stage_label_emphasis(projected)
        projected = re.sub(
            r"(?<![\w\\])(?:\*{1,3}|_{1,3})(?=\S)",
            "",
            projected,
        )
        projected = re.sub(
            r"(?<=\S)(?:\*{1,3}|_{1,3})(?!\w)",
            "",
            projected,
        )
        return " ".join(projected.split())

    def stage3f_selected_status_sentences(source: str) -> tuple[str, ...]:
        return tuple(
            sentence.strip()
            for sentence in re.split(r"(?<=[.!?])[ \t]+", source)
            if sentence.strip()
            and (
                re.search(r"\bStage[- ]*3F\b", sentence, re.IGNORECASE)
                or (
                    re.search(r"\braw[- ]?177\b", sentence, re.IGNORECASE)
                    and re.search(
                        r"\b(?:receipt|run|artifact)\b",
                        sentence,
                        re.IGNORECASE,
                    )
                )
            )
        )

    def stage3g_selected_status_sentences(source: str) -> tuple[str, ...]:
        return tuple(
            sentence.strip()
            for sentence in re.split(r"(?<=[.!?])[ \t]+", source)
            if sentence.strip()
            and (
                re.search(r"\bStage[- ]*3G\b", sentence, re.IGNORECASE)
                or (
                    re.search(r"\braw[- ]?11\b", sentence, re.IGNORECASE)
                    and re.search(
                        r"\b(?:receipt|run|artifact)\b",
                        sentence,
                        re.IGNORECASE,
                    )
                )
            )
        )

    def stage3h_selected_status_sentences(source: str) -> tuple[str, ...]:
        return tuple(
            sentence.strip()
            for sentence in re.split(r"(?<=[.!?])[ \t]+", source)
            if sentence.strip()
            and (
                re.search(r"\bStage[- ]*3H\b", sentence, re.IGNORECASE)
                or (
                    re.search(r"\braw[- ]?111\b", sentence, re.IGNORECASE)
                    and re.search(
                        r"\b(?:receipt|run|artifact|fingerprint|reports?)\b",
                        sentence,
                        re.IGNORECASE,
                    )
                )
            )
        )

    def stage3i_selected_status_sentences(source: str) -> tuple[str, ...]:
        return tuple(
            sentence.strip()
            for sentence in re.split(r"(?<=[.!?])[ \t]+", source)
            if sentence.strip()
            and re.search(r"\bStage[- ]*3I\b", sentence, re.IGNORECASE)
        )

    def stage3j_selected_status_sentences(source: str) -> tuple[str, ...]:
        return tuple(
            sentence.strip()
            for sentence in re.split(r"(?<=[.!?])[ \t]+", source)
            if sentence.strip()
            and (
                re.search(r"\bStage[- ]*3J\b", sentence, re.IGNORECASE)
                or (
                    re.search(r"\braw[- ]?112\b", sentence, re.IGNORECASE)
                    and re.search(
                        r"\b(?:receipt|run|artifact|fingerprint|reports?)\b",
                        sentence,
                        re.IGNORECASE,
                    )
                )
            )
        )

    stage3f_default_ignorable_ranges = (
        (0x034F, 0x034F),
        (0x115F, 0x1160),
        (0x17B4, 0x17B5),
        (0x180B, 0x180F),
        (0x2065, 0x2065),
        (0x3164, 0x3164),
        (0xFE00, 0xFE0F),
        (0xFFA0, 0xFFA0),
        (0xFFF0, 0xFFF8),
        (0xE0000, 0xE0FFF),
    )

    def stage3f_is_default_ignorable(character: str) -> bool:
        if unicodedata.category(character) == "Cf":
            return True
        codepoint = ord(character)
        return any(
            start <= codepoint <= end
            for start, end in stage3f_default_ignorable_ranges
        )

    def stage3f_html_attributes(
        attributes: str,
    ) -> list[tuple[str, str | None]]:
        parsed: list[tuple[str, str | None]] = []
        index = 0
        while index < len(attributes):
            while (
                index < len(attributes)
                and (attributes[index].isspace() or attributes[index] == "/")
            ):
                index += 1
            if index >= len(attributes):
                break

            name_start = index
            while (
                index < len(attributes)
                and not attributes[index].isspace()
                and attributes[index] not in {'"', "'", "<", ">", "/", "="}
            ):
                index += 1
            if name_start == index:
                index += 1
                continue
            name = attributes[name_start:index].casefold()

            while index < len(attributes) and attributes[index].isspace():
                index += 1
            value: str | None = None
            if index < len(attributes) and attributes[index] == "=":
                index += 1
                while index < len(attributes) and attributes[index].isspace():
                    index += 1
                if index < len(attributes) and attributes[index] in {'"', "'"}:
                    quote = attributes[index]
                    index += 1
                    value_start = index
                    while index < len(attributes) and attributes[index] != quote:
                        index += 1
                    value = attributes[value_start:index]
                    if index < len(attributes):
                        index += 1
                else:
                    value_start = index
                    while (
                        index < len(attributes)
                        and not attributes[index].isspace()
                        and attributes[index] != ">"
                    ):
                        index += 1
                    value = attributes[value_start:index]
            parsed.append((name, value))
        return parsed

    def stage3f_reference_label(label: str) -> str:
        return " ".join(label.split()).casefold()

    def stage3f_forensic_text_projection(source: str) -> str:
        comment_content: list[str] = []
        index = 0
        while index < len(source):
            if source.startswith("<!--", index):
                comment_end = source.find("-->", index + 4)
                if comment_end < 0:
                    comment_content.append(source[index + 4:])
                    break
                comment_content.append(source[index + 4:comment_end])
                index = comment_end + 3
                continue
            comment_content.append(source[index])
            index += 1

        without_comments = "".join(comment_content)
        rendered: list[str] = []
        index = 0
        while index < len(without_comments):
            if without_comments[index] == "<":
                cursor = index + 1
                closing_tag = (
                    cursor < len(without_comments)
                    and without_comments[cursor] == "/"
                )
                if closing_tag:
                    cursor += 1
                while (
                    cursor < len(without_comments)
                    and without_comments[cursor].isspace()
                ):
                    cursor += 1
                tag_match = re.match(
                    r"[A-Za-z][A-Za-z0-9:-]*",
                    without_comments[cursor:],
                )
                if tag_match is not None:
                    tag_name = tag_match.group(0).lower()
                    cursor += len(tag_match.group(0))
                    attributes_start = cursor
                    quote: str | None = None
                    while cursor < len(without_comments):
                        character = without_comments[cursor]
                        if quote is not None:
                            if character == quote:
                                quote = None
                        elif character in {"'", '"'}:
                            quote = character
                        elif character == ">":
                            if tag_name == "img" and not closing_tag:
                                attributes = without_comments[
                                    attributes_start:cursor
                                ]
                                for name, value in stage3f_html_attributes(
                                    attributes
                                ):
                                    if name == "alt":
                                        rendered.append(value or "")
                                        break
                            index = cursor + 1
                            break
                        cursor += 1
                    else:
                        index = len(without_comments)
                    continue
            rendered.append(without_comments[index])
            index += 1

        projected = unicodedata.normalize(
            "NFKC",
            html.unescape("".join(rendered)),
        )
        projected = "".join(
            character
            for character in projected
            if not stage3f_is_default_ignorable(character)
        )
        reference_definitions = {
            stage3f_reference_label(match.group("label"))
            for match in re.finditer(
                r"(?m)^[ \t]{0,3}\[(?P<label>[^\]\n]+)\]:"
                r"[ \t]*(?:<[^>\n]+>|\S+)",
                projected,
            )
        }
        projected = " ".join(projected.split())

        code_span = re.compile(
            r"(?P<ticks>`+)(?P<content>.*?)(?P=ticks)",
        )
        projected = code_span.sub(
            lambda match: match.group("content"),
            projected,
        )
        projected = re.sub(
            r"!\[([^]\n]*)\]\([^)]*\)",
            r"\1",
            projected,
        )
        projected = re.sub(
            r"!\[([^]\n]*)\]\[[^]\n]*\]",
            r"\1",
            projected,
        )
        projected = re.sub(
            r"!\[(?P<label>[^]\n]+)\](?![\[(])",
            lambda match: (
                match.group("label")
                if stage3f_reference_label(match.group("label"))
                in reference_definitions
                else match.group(0)
            ),
            projected,
        )
        projected = re.sub(
            r"\[([^]\n]+)\]\([^)]*\)",
            r"\1",
            projected,
        )
        projected = re.sub(
            r"\[([^]\n]+)\]\[[^]\n]*\]",
            r"\1",
            projected,
        )
        projected = projected.replace("~~", "")
        projected = stage3f_strip_inline_stage_label_emphasis(projected)
        projected = re.sub(
            r"(?<![\w\\])(?:\*{1,3}|_{1,3})(?=\S)",
            "",
            projected,
        )
        projected = re.sub(
            r"(?<=\S)(?:\*{1,3}|_{1,3})(?!\w)",
            "",
            projected,
        )
        return " ".join(projected.split())

    normalized_stage3f_status = " ".join(stage3e_status.split())
    rendered_stage3f_status = stage3f_rendered_text_projection(stage3e_status)
    forensic_stage3f_status = stage3f_forensic_text_projection(stage3e_status)
    stage3f_source_current_negation = re.compile(
        r"\b(?:not|never|no[ \t]+longer)\b[^.;!?]{0,32}\bsource[- ]current\b",
        re.IGNORECASE,
    )
    stage3f_stale_or_pending_claim = re.compile(
        r"\bStage[- ]*3F\b[^.;!?]{0,100}\b(?:source[- ]ahead|source[- ]stale|"
        r"unauthenticated|uncertified|pending|awaiting)\b",
        re.IGNORECASE,
    )
    if stage3f_source_current_negation.search(forensic_stage3f_status):
        fail(
            "stage3f-status",
            "the promoted Stage3F source-current status must not be negated",
        )
    if stage3f_stale_or_pending_claim.search(forensic_stage3f_status):
        fail(
            "stage3f-status",
            "the promoted Stage3F receipt must not be contradicted by a source-ahead, stale, unauthenticated, uncertified, pending, or awaiting claim",
        )

    stage3f_status_sentences = stage3f_selected_status_sentences(
        normalized_stage3f_status
    )
    stage3f_status_sentence_hash = hashlib.sha256(
        "\n".join(stage3f_status_sentences).encode("utf-8")
    ).hexdigest()
    if (
        len(stage3f_status_sentences) != 7
        or stage3f_status_sentence_hash
        != "a78dc4a9337047bf08987af495b9e94c5c372b1dbba0bbba077ddd779a86f941"
    ):
        fail(
            "stage3f-status",
            "the ordered canonical Stage3F status/provenance sentence inventory must remain exact; "
            f"found {len(stage3f_status_sentences)} sentences with sha256 {stage3f_status_sentence_hash}",
        )

    rendered_stage3f_status_sentences = stage3f_selected_status_sentences(
        rendered_stage3f_status
    )
    rendered_stage3f_status_sentence_hash = hashlib.sha256(
        "\n".join(rendered_stage3f_status_sentences).encode("utf-8")
    ).hexdigest()
    if (
        len(rendered_stage3f_status_sentences) != 7
        or rendered_stage3f_status_sentence_hash
        != "a78dc4a9337047bf08987af495b9e94c5c372b1dbba0bbba077ddd779a86f941"
    ):
        fail(
            "stage3f-status",
            "the ordered rendered-text Stage3F/status-provenance sentence inventory must remain exact after stripping HTML comments/tags and Markdown emphasis, decoding entities, and normalizing Unicode whitespace; "
            f"found {len(rendered_stage3f_status_sentences)} sentences with sha256 {rendered_stage3f_status_sentence_hash}",
        )

    forensic_stage3f_status_sentences = stage3f_selected_status_sentences(
        forensic_stage3f_status
    )
    forensic_stage3f_status_sentence_hash = hashlib.sha256(
        "\n".join(forensic_stage3f_status_sentences).encode("utf-8")
    ).hexdigest()
    if (
        len(forensic_stage3f_status_sentences) != 7
        or forensic_stage3f_status_sentence_hash
        != "b9ca8a2c28fdf291d794b59671f4fe25e10e6edef2c016af658e8b4741d62dff"
    ):
        fail(
            "stage3f-status",
            "the ordered forensic Stage3F/status-provenance sentence inventory must remain exact after retaining HTML comment content, stripping tags while preserving image alt text, decoding entities, NFKC normalization, removing default-ignorable formatting, flattening Markdown code/link/image labels, and stripping emphasis/strong/strikethrough delimiters; "
            f"found {len(forensic_stage3f_status_sentences)} sentences with sha256 {forensic_stage3f_status_sentence_hash}",
        )

    stage3g_source_current_negation = re.compile(
        r"\b(?:not|never|no[ \t]+longer)\b[^.;!?]{0,32}\bsource[- ]current\b",
        re.IGNORECASE,
    )
    stage3g_stale_or_pending_claim = re.compile(
        r"\bStage[- ]*3G\b[^.;!?]{0,100}\b(?:source[- ]ahead|source[- ]stale|"
        r"unauthenticated|uncertified|pending|awaiting)\b",
        re.IGNORECASE,
    )
    if stage3g_source_current_negation.search(forensic_stage3f_status):
        fail(
            "stage3g-status",
            "the promoted Stage3G source-current status must not be negated",
        )
    if stage3g_stale_or_pending_claim.search(forensic_stage3f_status):
        fail(
            "stage3g-status",
            "the promoted Stage3G receipt must not be contradicted by a source-ahead, stale, unauthenticated, uncertified, pending, or awaiting claim",
        )

    stage3g_status_projections = (
        (
            "canonical",
            normalized_stage3f_status,
            7,
            "e801d3e17488ad13105434f9859e65215cb598e18146db68cfb5baeb8c76a742",
        ),
        (
            "rendered-text",
            rendered_stage3f_status,
            7,
            "e801d3e17488ad13105434f9859e65215cb598e18146db68cfb5baeb8c76a742",
        ),
        (
            "forensic",
            forensic_stage3f_status,
            7,
            "b8c06fecd743a67dcf11acbc21fce1139b47f10beb4ed812b3b081abf4aa55f0",
        ),
    )
    for projection_name, projection, expected_count, expected_hash in stage3g_status_projections:
        sentences = stage3g_selected_status_sentences(projection)
        sentence_hash = hashlib.sha256(
            "\n".join(sentences).encode("utf-8")
        ).hexdigest()
        if len(sentences) != expected_count or sentence_hash != expected_hash:
            fail(
                "stage3g-status",
                "the ordered Stage3G/raw11 status sentence inventory must remain exact in the "
                f"{projection_name} projection; found {len(sentences)} sentences with sha256 {sentence_hash}",
            )

    stage3h_status_projections = (
        (
            "canonical",
            normalized_stage3f_status,
            8,
            "60c6cd7bebb26712f9fd4adb599d37e1a24d61116c5d20fcedf6d8aeba378139",
        ),
        (
            "rendered-text",
            rendered_stage3f_status,
            8,
            "60c6cd7bebb26712f9fd4adb599d37e1a24d61116c5d20fcedf6d8aeba378139",
        ),
        (
            "forensic",
            forensic_stage3f_status,
            8,
            "85712d3fe7a8732f1270937122c03468284c5c602c7cab503510d594be2751c9",
        ),
        (
            "forensic-confusable-skeleton",
            "".join(
                character
                for character in forensic_stage3f_status.translate(
                    str.maketrans(
                        {
                            "\u041d": "H",
                            "\u043d": "h",
                            "\u0397": "H",
                            "\u03b7": "h",
                        }
                    )
                )
                if unicodedata.category(character) not in {"Cc", "Cf"}
            ),
            8,
            "85712d3fe7a8732f1270937122c03468284c5c602c7cab503510d594be2751c9",
        ),
    )
    for projection_name, projection, expected_count, expected_hash in stage3h_status_projections:
        sentences = stage3h_selected_status_sentences(projection)
        sentence_hash = hashlib.sha256(
            "\n".join(sentences).encode("utf-8")
        ).hexdigest()
        if len(sentences) != expected_count or sentence_hash != expected_hash:
            fail(
                "stage3h-status",
                "the ordered Stage3H/raw111 status sentence inventory must remain exact in the "
                f"{projection_name} projection; found {len(sentences)} sentences with sha256 {sentence_hash}",
            )

    stage3h_source_current_negation = re.compile(
        r"\b(?:not|never|no[ \t]+longer)\b[^.;!?]{0,32}\bsource[- ]current\b",
        re.IGNORECASE,
    )
    stage3h_stale_or_pending_claim = re.compile(
        r"\bStage[- ]*3H\b[^.;!?]{0,100}\b(?:source[- ]ahead|source[- ]stale|"
        r"unauthenticated|uncertified|pending|awaiting)\b",
        re.IGNORECASE,
    )
    stage3h_forensic_claim_projections = (
        forensic_stage3f_status,
        stage3h_status_projections[-1][1],
    )
    if any(
        stage3h_source_current_negation.search(projection)
        for projection in stage3h_forensic_claim_projections
    ):
        fail(
            "stage3h-status",
            "the promoted Stage3H source-current status must not be negated",
        )
    if any(
        stage3h_stale_or_pending_claim.search(projection)
        for projection in stage3h_forensic_claim_projections
    ):
        fail(
            "stage3h-status",
            "the promoted receipt's inherited Stage3H coverage must not be contradicted by a source-ahead, stale, unauthenticated, uncertified, pending, or awaiting claim",
        )

    stage3i_status_projections = (
        (
            "canonical",
            normalized_stage3f_status,
            11,
            "5be8a9b239b1b6ea899d1522237a108e04ba4e13bf4e9b1e65ef160791fa9c37",
        ),
        (
            "rendered-text",
            rendered_stage3f_status,
            11,
            "5be8a9b239b1b6ea899d1522237a108e04ba4e13bf4e9b1e65ef160791fa9c37",
        ),
        (
            "forensic",
            forensic_stage3f_status,
            11,
            "d9ecb424b7fa27f3ada7eb1508c27979a5df6e22a9bef61572dc75e6cbb066af",
        ),
        (
            "forensic-confusable-skeleton",
            "".join(
                character
                for character in forensic_stage3f_status.translate(
                    str.maketrans(
                        {
                            "\u0406": "I",
                            "\u0456": "i",
                            "\u0399": "I",
                            "\u03b9": "i",
                        }
                    )
                )
                if unicodedata.category(character) not in {"Cc", "Cf"}
            ),
            11,
            "d9ecb424b7fa27f3ada7eb1508c27979a5df6e22a9bef61572dc75e6cbb066af",
        ),
    )
    for projection_name, projection, expected_count, expected_hash in stage3i_status_projections:
        sentences = stage3i_selected_status_sentences(projection)
        sentence_hash = hashlib.sha256(
            "\n".join(sentences).encode("utf-8")
        ).hexdigest()
        if len(sentences) != expected_count or sentence_hash != expected_hash:
            fail(
                "stage3i-status",
                "the ordered Stage3I/raw8 lifecycle sentence inventory must remain exact in the "
                f"{projection_name} projection; found {len(sentences)} sentences with sha256 {sentence_hash}",
            )

    stage3i_source_current_negation = re.compile(
        r"\b(?:not|never|no[ \t]+longer)\b[^.;!?]{0,32}\bsource[- ]current\b",
        re.IGNORECASE,
    )
    stage3i_stale_or_pending_claim = re.compile(
        r"\bStage[- ]*3I\b[^.;!?]{0,100}\b(?:source[- ]ahead|source[- ]stale|"
        r"unauthenticated|uncertified|pending|awaiting)\b",
        re.IGNORECASE,
    )
    stage3i_forensic_claim_projections = (
        forensic_stage3f_status,
        stage3i_status_projections[-1][1],
    )
    if any(
        stage3i_source_current_negation.search(projection)
        for projection in stage3i_forensic_claim_projections
    ):
        fail(
            "stage3i-status",
            "the promoted Stage3I source-current status must not be negated",
        )
    if any(
        stage3i_stale_or_pending_claim.search(projection)
        for projection in stage3i_forensic_claim_projections
    ):
        fail(
            "stage3i-status",
            "the promoted Stage3I receipt must not be contradicted by a source-ahead, stale, unauthenticated, uncertified, pending, or awaiting claim",
        )

    stage3j_status_projections = (
        (
            "canonical",
            normalized_stage3f_status,
            0,
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
        ),
        (
            "rendered-text",
            rendered_stage3f_status,
            0,
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
        ),
        (
            "forensic",
            forensic_stage3f_status,
            0,
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
        ),
        (
            "forensic-confusable-skeleton",
            "".join(
                character
                for character in forensic_stage3f_status.translate(
                    str.maketrans(
                        {
                            "\u0408": "J",
                            "\u0458": "j",
                        }
                    )
                )
                if unicodedata.category(character) not in {"Cc", "Cf"}
            ),
            0,
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
        ),
    )
    for projection_name, projection, expected_count, expected_hash in stage3j_status_projections:
        sentences = stage3j_selected_status_sentences(projection)
        sentence_hash = hashlib.sha256(
            "\n".join(sentences).encode("utf-8")
        ).hexdigest()
        if len(sentences) != expected_count or sentence_hash != expected_hash:
            fail(
                "stage3j-status",
                "Stage3J/raw112 lifecycle claims must remain absent until the admission exists in the "
                f"{projection_name} projection; found {len(sentences)} sentences with sha256 {sentence_hash}",
            )

    stage3j_contrary_claims = (
        r"\bStage[- ]*3J\b[^.;!?]{0,100}\b(?:is|remains)[ \t]+source[- ]current\b",
        r"\bStage[- ]*3J\b[^.;!?]{0,100}\b(?:is|has[ \t]+been|remains)[ \t]+(?:authenticated|certified|covered)\b",
        r"\b(?:this[ \t]+)?(?:promoted[ \t]+)?receipt\b[^.;!?]{0,100}\b(?:authenticates|certifies|covers)\b[^.;!?]{0,80}\b(?:Stage[- ]*3J|raw[- ]?112)\b",
        r"\b(?:run|job|artifact|fingerprint|reports?)\b[^.;!?]{0,100}\b(?:authenticates|certifies|covers)\b[^.;!?]{0,80}\b(?:Stage[- ]*3J|raw[- ]?112)\b",
        r"\bStage[- ]*3J\b[^.;!?]{0,100}\b(?:not|never|no[ \t]+longer)[^.;!?]{0,32}\bsource[- ]ahead\b",
        r"\b(?:not|never|no[ \t]+longer)[^.;!?]{0,32}\bsource[- ]ahead\b[^.;!?]{0,100}\bStage[- ]*3J\b",
        r"\bStage[- ]*3J\b[^.;!?]{0,120}\b(?:adds|makes|establishes)[ \t]+(?:a[ \t]+)?new[ \t]+(?:Test262[ \t]+)?conformance\b",
    )
    stage3j_forensic_claim_projections = (
        forensic_stage3f_status,
        stage3j_status_projections[-1][1],
    )
    if any(
        re.search(pattern, projection, re.IGNORECASE)
        for projection in stage3j_forensic_claim_projections
        for pattern in stage3j_contrary_claims
    ):
        fail(
            "stage3j-status",
            "Stage3J/raw112 source-current, receipt-covered, authenticated, certified, source-ahead-negating, or new-conformance claims are forbidden until the admission exists",
        )

    stage3e_receipt_paragraphs = [
        paragraph
        for paragraph in re.split(r"\n[ \t]*\n", stage3e_status)
        if "The latest full R3fj execution" in paragraph
    ]
    stage3e_receipt_config = read_source("dev-support/test262/current.conf")
    stage3e_receipt_values: dict[str, str] = {}
    for line_number, line in enumerate(stage3e_receipt_config.splitlines(), 1):
        if not line or line.startswith("#"):
            continue
        key, separator, value = line.partition("=")
        if (
            separator != "="
            or not re.fullmatch(r"[a-z][a-z0-9_]*", key)
            or not value
        ):
            fail(
                "stage3i-status",
                f"dev-support/test262/current.conf:{line_number} is not a canonical key=value receipt field",
            )
            continue
        if key in stage3e_receipt_values:
            fail(
                "stage3i-status",
                f"dev-support/test262/current.conf repeats receipt field {key}",
            )
            continue
        stage3e_receipt_values[key] = value

    stage3e_receipt_metrics = {
        "milestone": "r3fj",
        "focused_variants": "6844",
        "focused_eligible": "6844",
        "focused_runnable": "6844",
        "focused_passes": "6844",
        "focused_tsv_lines": "6857",
        "focused_jsonl_lines": "6846",
        "focused_summary": "pass=6844",
        "full_variants": "102037",
        "full_eligible": "80032",
        "full_runnable": "80032",
        "full_passes": "79982",
        "full_tsv_lines": "102050",
        "full_jsonl_lines": "102039",
        "full_summary": "fail-parse=7 fail-runtime=43 pass=79982 skipped-config-exclude=6700 skipped-feature=11775 unsupported-feature=847 unsupported-module=121 unsupported-negative-provenance=2562",
    }
    drifted_stage3e_receipt_metrics = {
        key: stage3e_receipt_values.get(key)
        for key, expected in stage3e_receipt_metrics.items()
        if stage3e_receipt_values.get(key) != expected
    }
    if drifted_stage3e_receipt_metrics:
        fail(
            "stage3i-status",
            "the promoted Stage3I receipt must retain the exact unchanged R3fj focused/full metrics; "
            f"found {drifted_stage3e_receipt_metrics}",
        )

    stage3e_receipt_hex_fields = {
        "engine_semantics_source": 40,
        "engine_semantics_sha256": 64,
        "focused_tsv_sha256": 64,
        "focused_jsonl_sha256": 64,
        "full_tsv_sha256": 64,
        "full_jsonl_sha256": 64,
    }
    invalid_stage3e_receipt_fields = {
        key: stage3e_receipt_values.get(key)
        for key, width in stage3e_receipt_hex_fields.items()
        if not re.fullmatch(
            rf"[0-9a-f]{{{width}}}", stage3e_receipt_values.get(key, "")
        )
    }
    if invalid_stage3e_receipt_fields:
        fail(
            "stage3i-status",
            "the promoted Stage3I receipt source, fingerprint, and focused/full report hashes must be canonical current.conf fields; "
            f"found {invalid_stage3e_receipt_fields}",
        )

    stage3e_promoted_receipt_values = {
        "engine_semantics_source": "022e7b4860ec9b6e6d2922f835ac694790880126",
        "engine_semantics_sha256": "f61afc7314c09e4b507468ca9bffdeb920d3e9c896068b3a9f39b4587caa0333",
        "focused_tsv_sha256": "fec3395e614b678fefcde53e880605126dfed3aab58e2c4e69cc046d70252b98",
        "focused_jsonl_sha256": "f8aa7b03998c11d8cc29ae84bb9a8dcad512ac6d368af8819ed6bf9f7b9144d2",
        "full_tsv_sha256": "52a4288752e11855a5101c7a536717690bdcb2d582224defc58bbe1ebfd3da91",
        "full_jsonl_sha256": "6886651883250c460dde168d85d1d44524f3b9f4dc5398859a0c2071604e6a3d",
    }
    drifted_stage3e_promoted_receipt_values = {
        key: stage3e_receipt_values.get(key)
        for key, expected in stage3e_promoted_receipt_values.items()
        if stage3e_receipt_values.get(key) != expected
    }
    if drifted_stage3e_promoted_receipt_values:
        fail(
            "stage3i-receipt-pin",
            "current.conf must retain the exact reviewed Stage3I source, fingerprint, and focused/full receipt hashes; "
            f"found {drifted_stage3e_promoted_receipt_values}",
        )

    stage3e_focused_receipt_paths = {
        "focused_tsv": (
            "tests/test262-class-private-callables-b-global-candidate.tsv",
            "focused_tsv_sha256",
            1543969,
        ),
        "focused_jsonl": (
            "tests/test262-class-private-callables-b-global-candidate.jsonl",
            "focused_jsonl_sha256",
            3501190,
        ),
    }
    resolved_root = root.resolve()
    for path_key, (expected_relative, hash_key, expected_bytes) in stage3e_focused_receipt_paths.items():
        configured_relative = stage3e_receipt_values.get(path_key)
        if configured_relative != expected_relative:
            fail(
                "stage3i-focused-receipt",
                f"current.conf {path_key} must resolve the exact reviewed path {expected_relative}; found {configured_relative!r}",
            )
            continue
        candidate = root / configured_relative
        expected_lexical_path = resolved_root / expected_relative
        try:
            resolved_candidate = candidate.resolve(strict=True)
        except (OSError, RuntimeError) as error:
            fail(
                "stage3i-focused-receipt",
                f"{expected_relative} must resolve as the reviewed focused receipt: {error}",
            )
            continue
        if (
            candidate.is_symlink()
            or not candidate.is_file()
            or resolved_candidate != expected_lexical_path
            or candidate.stat().st_nlink != 1
            or candidate.stat().st_size != expected_bytes
        ):
            fail(
                "stage3i-focused-receipt",
                f"{expected_relative} must be the exact {expected_bytes}-byte regular, non-symlink, single-link reviewed focused receipt",
            )
            continue
        actual_hash = hashlib.sha256(candidate.read_bytes()).hexdigest()
        configured_hash = stage3e_receipt_values.get(hash_key)
        if actual_hash != configured_hash:
            fail(
                "stage3i-focused-receipt",
                f"{expected_relative} bytes must match current.conf {hash_key}; found {actual_hash}",
            )

    if len(stage3e_receipt_paragraphs) != 1:
        fail(
            "stage3i-status",
            "the status document must contain exactly one promoted Stage3I R3fj receipt paragraph with a blank-line boundary",
        )
    elif not invalid_stage3e_receipt_fields and not drifted_stage3e_receipt_metrics:
        stage3e_receipt_offset = stage3e_status.find(stage3e_receipt_paragraphs[0])
        stage3e_receipt_prefix = stage3e_status[:stage3e_receipt_offset]
        stage3e_receipt_is_indented_code = any(
            re.match(r"(?:[ ]{4,}|\t)", line)
            for line in stage3e_receipt_paragraphs[0].splitlines()
            if line
        )
        active_stage3e_fence: tuple[str, int] | None = None
        for line in stage3e_receipt_prefix.splitlines():
            fence = re.match(
                r"^[ ]{0,3}(?P<marker>`{3,}|~{3,})(?P<tail>.*)$", line
            )
            if fence is None:
                continue
            marker = fence.group("marker")
            tail = fence.group("tail")
            if active_stage3e_fence is None:
                if marker[0] != "`" or "`" not in tail:
                    active_stage3e_fence = (marker[0], len(marker))
            elif (
                marker[0] == active_stage3e_fence[0]
                and len(marker) >= active_stage3e_fence[1]
                and not tail.strip()
            ):
                active_stage3e_fence = None

        stage3e_void_html_tags = {
            "area", "base", "br", "col", "embed", "hr", "img", "input",
            "link", "meta", "param", "source", "track", "wbr",
        }

        def raw_html_ancestor_stack_at(source: str, stop: int) -> tuple[str, ...]:
            stack: list[str] = []
            index = 0
            while index < stop:
                opening = source.find("<", index, stop)
                if opening < 0:
                    break
                if source.startswith("<!--", opening):
                    comment_end = source.find("-->", opening + 4)
                    if comment_end < 0 or comment_end + 3 > stop:
                        stack.append("!--")
                        break
                    index = comment_end + 3
                    continue

                cursor = opening + 1
                closing = cursor < stop and source[cursor] == "/"
                if closing:
                    cursor += 1
                while cursor < stop and source[cursor].isspace():
                    cursor += 1
                tag_match = re.match(r"[A-Za-z][A-Za-z0-9:-]*", source[cursor:stop])
                if tag_match is None:
                    index = opening + 1
                    continue
                tag = tag_match.group(0).lower()
                cursor += len(tag_match.group(0))
                quote: str | None = None
                tag_end = -1
                while cursor < len(source):
                    character = source[cursor]
                    if quote is not None:
                        if character == quote:
                            quote = None
                    elif character in {"'", '"'}:
                        quote = character
                    elif character == ">":
                        tag_end = cursor
                        break
                    cursor += 1
                if tag_end < 0 or tag_end >= stop:
                    stack.append("!incomplete-tag")
                    break

                tag_tail = source[opening + 1:tag_end]
                if closing:
                    if stack and stack[-1] == tag:
                        stack.pop()
                    else:
                        stack.append(f"!mispaired:/{tag}")
                elif (
                    tag not in stage3e_void_html_tags
                    and not tag_tail.rstrip().endswith("/")
                ):
                    stack.append(tag)
                index = tag_end + 1
            return tuple(stack)

        stage3e_receipt_end = (
            stage3e_receipt_offset + len(stage3e_receipt_paragraphs[0])
        )
        stage3e_receipt_html_at_start = raw_html_ancestor_stack_at(
            stage3e_status,
            stage3e_receipt_offset,
        )
        stage3e_receipt_html_at_end = raw_html_ancestor_stack_at(
            stage3e_status,
            stage3e_receipt_end,
        )
        if (
            stage3e_receipt_is_indented_code
            or active_stage3e_fence is not None
            or stage3e_receipt_html_at_start
            or stage3e_receipt_html_at_end
        ):
            fail(
                "stage3i-status",
                "the promoted Stage3I R3fj receipt paragraph must remain top-level rendered Markdown, not a comment, code block, or raw-HTML descendant at either boundary",
            )

        stage3e_receipt_provenance = {
            "run": "32497291807",
            "job": "96818622699",
            "artifact": "9452593259",
            "artifact_sha256": "97ff9d79784ed27ec2a323544597b28aa2c5b5227b0028e5d3560bbaa22b1bfb",
            "stage3h_fingerprint": "d21943622773d2b0b978cd2ace5261d5ec41a9400ab36864768470aae71b1d22",
            "stage3h_run": "32419997996",
            "stage3h_artifact": "9425844939",
            "stage3h_artifact_sha256": "2c8dc920428aef4f10be440d7d18fdf72ec0902af4c7482f5be34ed4f25b1215",
        }
        source = re.escape(stage3e_receipt_values["engine_semantics_source"])
        fingerprint = re.escape(stage3e_receipt_values["engine_semantics_sha256"])
        focused_tsv_hash = re.escape(stage3e_receipt_values["focused_tsv_sha256"])
        focused_jsonl_hash = re.escape(stage3e_receipt_values["focused_jsonl_sha256"])
        full_tsv_hash = re.escape(stage3e_receipt_values["full_tsv_sha256"])
        full_jsonl_hash = re.escape(stage3e_receipt_values["full_jsonl_sha256"])
        focused_passes = f'{int(stage3e_receipt_values["focused_passes"]):,}'
        full_variants = f'{int(stage3e_receipt_values["full_variants"]):,}'
        full_eligible = f'{int(stage3e_receipt_values["full_eligible"]):,}'
        full_passes = f'{int(stage3e_receipt_values["full_passes"]):,}'
        full_tsv_lines = f'{int(stage3e_receipt_values["full_tsv_lines"]):,}'
        full_jsonl_lines = f'{int(stage3e_receipt_values["full_jsonl_lines"]):,}'
        receipt_pattern = re.compile(
            rf"The latest full R3fj execution, exact-source GitHub Actions run `{stage3e_receipt_provenance['run']}`, "
            rf"job `{stage3e_receipt_provenance['job']}`, authenticates Stage 3I source `{source}` with engine fingerprint `{fingerprint}`\. "
            rf"The canonical `test262-receipt` run completed successfully after its "
            rf"current-source gate accepted the expected one-fingerprint receipt refresh; its "
            rf"unique exact six-file artifact `{stage3e_receipt_provenance['artifact']}` \(SHA-256 "
            rf"`{stage3e_receipt_provenance['artifact_sha256']}`\) records the "
            rf"{full_tsv_lines}-line TSV as `{full_tsv_hash}` and the {full_jsonl_lines}-line JSONL as "
            rf"`{full_jsonl_hash}`\. Each full receipt contains the Stage 3I fingerprint exactly once and no Stage "
            rf"3H fingerprint\. Fingerprint-only normalization replaces that single occurrence "
            rf"with the authenticated Stage 3H fingerprint `{stage3e_receipt_provenance['stage3h_fingerprint']}` "
            rf"and makes both files byte-for-byte "
            rf"identical to Stage 3H run `{stage3e_receipt_provenance['stage3h_run']}`, "
            rf"artifact `{stage3e_receipt_provenance['stage3h_artifact']}` \(SHA-256 "
            rf"`{stage3e_receipt_provenance['stage3h_artifact_sha256']}`\): "
            rf"all {full_variants} classified outcomes, {full_passes} full passes, and {full_eligible} eligible variants "
            rf"are unchanged\. The refreshed {focused_passes}-pass focused TSV and JSONL are "
            rf"byte-identical on exact-source replay at hashes `{focused_tsv_hash}` and `{focused_jsonl_hash}`\. "
            rf"This promoted receipt is source-current for Stage 3I and covers the raw-8 "
            rf"`PushThis` admission and its Rust/C evidence without changing the Test262 "
            rf"profile or any focused or full metric reported above\. It remains the exact "
            rf"Stage-3I lifecycle boundary, retains the Stage-3H raw-111 `ToObject`, Stage-3G "
            rf"raw-11 Object, and Stage-3F "
            rf"raw-177 coverage, and makes no new conformance claim\."
        )
        normalized_receipt_paragraph = " ".join(
            stage3e_receipt_paragraphs[0].split()
        )
        if receipt_pattern.fullmatch(normalized_receipt_paragraph) is None:
            fail(
                "stage3i-status",
                "the complete promoted Stage3I R3fj paragraph must match current.conf source, fingerprint, focused/full report hashes, exact successful provenance, normalized Stage3H equality, inherited Stage3H/Stage3G/Stage3F coverage, unchanged metrics, and its blank-line boundary",
            )

    normalized_stage3e_status = " ".join(stage3e_status.split())
    stage3e_contrary_claims = (
        r"\b(?:source[- ]stale|stale)[ \t]+for[ \t]+Stage[ \t]*3I\b",
        r"\b(?:does[ \t]+not|cannot)[ \t]+(?:certify|cover|authenticate)[ \t]+(?:the[ \t]+)?(?:raw[- ]?8|Stage[ \t]*3I)\b",
        r"\bStage[ \t]*3I\b[^.]{0,100}\b(?:is|remains)[ \t]+(?:source[- ]ahead|source[- ]stale|stale|uncertified|unauthenticated|not[ \t]+authenticated|not[ \t]+certified)\b",
        r"\bStage[ \t]*3I\b[ \t]+(?:is|remains)[ \t]+(?:uncovered|not[ \t]+covered)\b",
        r"\bStage[ \t]*3I\b[^.]{0,100}\bhas[ \t]+(?:not[ \t]+been|yet[ \t]+to[ \t]+be)[ \t]+(?:authenticated|certified|covered)\b",
        r"\bStage[ \t]*3I\b[^.]{0,100}\b(?:pending|awaiting)[ \t]+(?:a[ \t]+)?(?:separate[ \t]+)?(?:exact-source[ \t]+)?receipt[ \t]+promotion\b",
        r"\bonly[ \t]+Stage[ \t]*3H\b[^.]{0,120}\b(?:is|was|has[ \t]+been)[ \t]+(?:authenticated|certified|covered)\b[^.]{0,120}\b(?:this[ \t]+)?receipt\b",
        r"\b(?:this[ \t]+)?receipt\b[^.]{0,100}\b(?:authenticates|certifies|covers)[ \t]+only[ \t]+Stage[ \t]*3H\b",
        r"\b(?:this[ \t]+)?receipt\b[ \t]+only[ \t]+(?:authenticates|certifies|covers)[ \t]+Stage[ \t]*3H\b",
        r"\b(?:source[- ]stale|stale)[ \t]+for[ \t]+Stage[ \t]*3H\b",
        r"\b(?:does[ \t]+not|cannot)[ \t]+(?:certify|cover|authenticate)[ \t]+(?:the[ \t]+)?(?:raw[- ]?111|Stage[ \t]*3H)\b",
        r"\bStage[ \t]*3H\b[^.]{0,100}\b(?:is|remains)[ \t]+(?:source[- ]ahead|source[- ]stale|stale|uncertified|unauthenticated|not[ \t]+authenticated|not[ \t]+certified)\b",
        r"\bStage[ \t]*3H\b[ \t]+(?:is|remains)[ \t]+(?:uncovered|not[ \t]+covered)\b",
        r"\bStage[ \t]*3H\b[^.]{0,100}\bhas[ \t]+(?:not[ \t]+been|yet[ \t]+to[ \t]+be)[ \t]+(?:authenticated|certified|covered)\b",
        r"\bStage[ \t]*3H\b[^.]{0,100}\b(?:pending|awaiting)[ \t]+(?:a[ \t]+)?(?:separate[ \t]+)?(?:exact-source[ \t]+)?receipt[ \t]+promotion\b",
        r"\bonly[ \t]+Stage[ \t]*3G\b[^.]{0,120}\b(?:is|was|has[ \t]+been)[ \t]+(?:authenticated|certified|covered)\b[^.]{0,120}\b(?:this[ \t]+)?receipt\b",
        r"\b(?:this[ \t]+)?receipt\b[^.]{0,100}\b(?:authenticates|certifies|covers)[ \t]+only[ \t]+Stage[ \t]*3G\b",
        r"\b(?:this[ \t]+)?receipt\b[ \t]+only[ \t]+(?:authenticates|certifies|covers)[ \t]+Stage[ \t]*3G\b",
        r"\b(?:source[- ]stale|stale)[ \t]+for[ \t]+Stage[ \t]*3G\b",
        r"\b(?:does[ \t]+not|cannot)[ \t]+(?:certify|cover|authenticate)[ \t]+(?:the[ \t]+)?(?:raw[- ]?11|Stage[ \t]*3G)\b",
        r"\bStage[ \t]*3G\b[^.]{0,100}\b(?:is|remains)[ \t]+(?:source[- ]ahead|source[- ]stale|stale|uncertified|unauthenticated|not[ \t]+authenticated|not[ \t]+certified)\b",
        r"\bStage[ \t]*3G\b[ \t]+(?:is|remains)[ \t]+(?:uncovered|not[ \t]+covered)\b",
        r"\bStage[ \t]*3G\b[^.]{0,100}\bhas[ \t]+(?:not[ \t]+been|yet[ \t]+to[ \t]+be)[ \t]+(?:authenticated|certified|covered)\b",
        r"\bStage[ \t]*3G\b[^.]{0,100}\b(?:pending|awaiting)[ \t]+(?:a[ \t]+)?(?:separate[ \t]+)?(?:exact-source[ \t]+)?receipt[ \t]+promotion\b",
        r"\bonly[ \t]+Stage[ \t]*3F\b[^.]{0,120}\b(?:is|was|has[ \t]+been)[ \t]+(?:authenticated|certified|covered)\b[^.]{0,120}\b(?:this[ \t]+)?receipt\b",
        r"\b(?:this[ \t]+)?receipt\b[^.]{0,100}\b(?:authenticates|certifies|covers)[ \t]+only[ \t]+Stage[ \t]*3F\b",
        r"\b(?:this[ \t]+)?receipt\b[ \t]+only[ \t]+(?:authenticates|certifies|covers)[ \t]+Stage[ \t]*3F\b",
        r"\b(?:source[- ]stale|stale)[ \t]+for[ \t]+Stage[ \t]*3F\b",
        r"\b(?:does[ \t]+not|cannot)[ \t]+(?:certify|cover|authenticate)[ \t]+(?:the[ \t]+)?(?:raw[- ]?177|Stage[ \t]*3F)\b",
        r"\bStage[ \t]*3F\b[^.]{0,100}\b(?:is|remains)[ \t]+(?:source[- ]stale|stale|uncertified|unauthenticated|not[ \t]+authenticated|not[ \t]+certified|uncovered|not[ \t]+covered)\b",
        r"\b(?:source[- ]stale|stale)[ \t]+for[ \t]+Stage[ \t]*3E\b",
        r"\b(?:does[ \t]+not|cannot)[ \t]+(?:certify|cover|authenticate)[ \t]+(?:the[ \t]+)?(?:raw[- ]?49|Stage[ \t]*3E)\b",
        r"\bStage[ \t]*3E\b[^.]{0,100}\b(?:is|remains)[ \t]+(?:source[- ]stale|stale|uncertified|unauthenticated|not[ \t]+authenticated|not[ \t]+certified|uncovered|not[ \t]+covered)\b",
    )
    if any(
        re.search(pattern, normalized_stage3e_status, re.IGNORECASE)
        for pattern in stage3e_contrary_claims
    ):
        fail(
            "stage3i-status",
            "the promoted Stage3I receipt must not be contradicted by an appended source-ahead, stale, uncertified, uncovered, pending-promotion, Stage3H-only, Stage3G-only, or Stage3F-only claim",
        )

src_root = root / "src"
if src_root.is_symlink() or not src_root.is_dir():
    fail("missing-source", "src must be a regular directory")
    production_sources: list[Path] = []
else:
    production_sources = sorted(src_root.rglob("*.rs"))

allowed_assertion_namespace_imports = {
    "use std::panic::{AssertUnwindSafe, catch_unwind, resume_unwind};",
    "use std::panic::{self, AssertUnwindSafe};",
}
for path in production_sources:
    if path.is_symlink() or not path.is_file():
        continue
    relative = path.relative_to(root).as_posix()
    code = rust_code_only(path.read_text(encoding="utf-8"))
    if re.search(
        r"\bmacro_rules[ \t\n]*![ \t\n]*(?:r#)?(?:assert|assert_eq|assert_ne|matches|panic)\b",
        code,
    ):
        fail(
            "stage3e-runtime-evidence",
            f"{relative} must not define an assertion macro that can enter Stage3E test scope",
        )
    for statement in re.findall(
        r"(?ms)^[ \t]*(?:(?:pub(?:[ \t]*\([^)]*\))?)[ \t]+)?use\b.*?;",
        code,
    ):
        if (
            assertion_shadow_pattern.search(statement)
            and " ".join(statement.split()) not in allowed_assertion_namespace_imports
        ):
            fail(
                "stage3e-runtime-evidence",
                f"{relative} must not import an assertion macro that can shadow Stage3E test evidence",
            )

facade_name_pattern = re.compile(
    r"\b(?:ScalarValueDraft|ScalarUnaryOp|ScalarScriptReadError|ScalarStringDraft|"
    r"decode_trusted_scalar_script|DetachedAtomName|DetachedPrimitive|OrdinaryLeafDraft|"
    r"OrdinaryLeafApplyKind|OrdinaryLeafMetadataDraft|OrdinaryLeafOp|OrdinaryLeafReadError|"
    r"RootFunctionConstantSelector|decode_trusted_ordinary_leaf)\b"
)
for path in production_sources:
    if path.is_symlink() or not path.is_file():
        continue
    relative = path.relative_to(root).as_posix()
    if relative.startswith("src/runtime/binary_object/"):
        continue
    source = path.read_text(encoding="utf-8")
    code = rust_code_only(source)
    binary_mentions = list(re.finditer(r"\bbinary_object\b", code))
    if relative == "src/runtime.rs":
        if len(binary_mentions) != 1:
            fail(
                "binary-object-consumer-set",
                "src/runtime.rs may name binary_object only in its private module declaration",
            )
    elif relative == consumer_relative and consumer_exists:
        if len(binary_mentions) != 1:
            fail(
                "binary-object-consumer-set",
                f"{consumer_relative} must remain the sole reviewed codec consumer",
            )
    elif binary_mentions:
        for match in binary_mentions:
            fail(
                "binary-object-consumer-set",
                "only binary_object_publish.rs may consume binary_object; found "
                + location(relative, source, match.start()),
            )

    if relative not in {binary_root_relative, consumer_relative}:
        for match in facade_name_pattern.finditer(code):
            fail(
                "binary-object-facade-consumer-set",
                "only binary_object_publish.rs may name the scalar-script or ordinary-leaf facade; found "
                + location(relative, source, match.start()),
            )

    if relative not in {bytecode_publish_relative, consumer_relative}:
        for match in re.finditer(r"\bverify_unlinked_ordinary_leaf\b", code):
            fail(
                "ordinary-leaf-verifier-consumer-set",
                "only binary_object_publish.rs may call the dedicated ordinary-leaf verifier; found "
                + location(relative, source, match.start()),
            )

    path_attribute = re.compile(
        r"#[ \t\n]*\[[ \t\n]*path[ \t\n]*=[ \t\n]*[^\]]*binary_object[^\]]*\]"
    )
    for match in path_attribute.finditer(source):
        fail(
            "binary-object-consumer-set",
            "path attributes must not create an alternate binary_object consumer; found "
            + location(relative, source, match.start()),
        )

cursor_relative = "src/runtime/binary_object/read_cursor.rs"
cursor_source = read_source(cursor_relative)
cursor_code = rust_code_only(cursor_source)
sealed_modules = re.findall(r"(?m)^[ \t]*mod[ \t]+sealed[ \t]*\{", cursor_code)
if len(sealed_modules) != 1 or re.search(
    r"\bpub(?:[ \t\n]*\([^)]*\))?[ \t\n]+mod[ \t\n]+sealed\b",
    cursor_code,
):
    fail(
        "common-cursor-unsealed",
        f"{cursor_relative} must contain exactly one private `mod sealed`",
    )

checked_trait_pattern = re.compile(
    r"\bpub[ \t\n]*\([ \t\n]*in[ \t\n]+crate[ \t\n]*::[ \t\n]*runtime"
    r"[ \t\n]*::[ \t\n]*binary_object[ \t\n]*\)[ \t\n]+trait[ \t\n]+"
    r"CheckedReadCursor[ \t\n]*<[ \t\n]*'input[ \t\n]*>[ \t\n]*:"
    r"[ \t\n]*sealed[ \t\n]*::[ \t\n]*Sealed\b"
)
if len(checked_trait_pattern.findall(cursor_code)) != 1:
    fail(
        "common-cursor-unsealed",
        f"{cursor_relative} must declare one binary_object-private CheckedReadCursor sealed by sealed::Sealed",
    )

forbidden_cursor_capability = re.compile(
    r"\bu64\b|\ballows_shared_array_buffers\b|\brecord_shared_array_buffer\b"
)
for match in forbidden_cursor_capability.finditer(cursor_code):
    fail(
        "forbidden-common-cursor-capability",
        "CheckedReadCursor must not expose raw u64 or SAB capability hooks; found "
        + location(cursor_relative, cursor_source, match.start()),
    )

sealed_cursor_alias = re.compile(r"\bSealed[ \t\n]+as[ \t\n]+")
for match in sealed_cursor_alias.finditer(cursor_code):
    fail(
        "common-cursor-seal-alias",
        "the common cursor seal must not be renamed before an implementation; found "
        + location(cursor_relative, cursor_source, match.start()),
    )

checked_impl_pattern = re.compile(
    r"\bimpl\b(?P<header>[^{};]*\bCheckedReadCursor\b"
    r"[ \t\n]*(?:(?:::[ \t\n]*)?<[^{};>]*>)?[ \t\n]+for\b[^{};]*)\{",
    re.DOTALL,
)
checked_cursor_alias = re.compile(r"\bCheckedReadCursor[ \t\n]+as[ \t\n]+")
checked_impl_headers: list[tuple[str, str]] = []
for path in binary_sources:
    if path.is_symlink() or not path.is_file():
        continue
    relative = path.relative_to(root).as_posix()
    source = binary_source_cache[path]
    code = binary_code_cache[path]
    for match in checked_cursor_alias.finditer(code):
        fail(
            "common-cursor-trait-alias",
            "CheckedReadCursor must not be renamed before an implementation; found "
            + location(relative, source, match.start()),
        )
    for match in checked_impl_pattern.finditer(code):
        header = " ".join(("impl" + match.group("header") + " {").split())
        checked_impl_headers.append((relative, header))
expected_checked_impl_headers = [
    (
        cursor_relative,
        "impl<'input> CheckedReadCursor<'input> for SabTransportCursor<'input> {",
    ),
    (
        cursor_relative,
        "impl<'input> CheckedReadCursor<'input> for WireCursor<'input> {",
    ),
]
if sorted(checked_impl_headers) != sorted(expected_checked_impl_headers):
    fail(
        "common-cursor-implementation-set",
        "CheckedReadCursor must have only the two canonical implementations in read_cursor.rs; "
        f"found {checked_impl_headers}",
    )

sealed_impl_pattern = re.compile(
    r"\bimpl\b(?P<header>[^{};]*\bSealed\b"
    r"[ \t\n]*(?:::[ \t\n]*<[^{};>]*>)?[ \t\n]+for\b[^{};]*)\{",
    re.DOTALL,
)
sealed_impl_headers = [
    " ".join(("impl" + match.group("header") + " {").split())
    for match in sealed_impl_pattern.finditer(cursor_code)
]
expected_sealed_impl_headers = [
    "impl Sealed for SabTransportCursor<'_> {",
    "impl Sealed for WireCursor<'_> {",
]
if sorted(sealed_impl_headers) != sorted(expected_sealed_impl_headers):
    fail(
        "common-cursor-seal-implementation-set",
        "the private common cursor seal must have only the two canonical implementations; "
        f"found {sealed_impl_headers}",
    )

graph_decode_relative = "src/runtime/binary_object/graph/decode.rs"
image_decode_relative = "src/runtime/binary_object/bytecode_image/decode/mod.rs"
sab_transport_relative = "src/runtime/binary_object/graph/sab_transport.rs"
image_model_relative = "src/runtime/binary_object/bytecode_image/model.rs"
image_atoms_relative = "src/runtime/binary_object/bytecode_image/atoms.rs"
graph_decode_source = read_source(graph_decode_relative)
graph_decode_code = rust_code_only(graph_decode_source)
image_decode_source = read_source(image_decode_relative)
image_decode_code = rust_code_only(image_decode_source)
sab_transport_source = read_source(sab_transport_relative)
sab_transport_code = rust_code_only(sab_transport_source)
image_model_source = read_source(image_model_relative)
image_model_code = rust_code_only(image_model_source)
image_atoms_source = read_source(image_atoms_relative)
image_atoms_code = rust_code_only(image_atoms_source)
if (
    is_full_binary_inventory
    and normalized_code_sha256(image_model_code)
    != "a7ddad998b12ccd6f69e57aa0c57d124a24077d3e88c44e649a0d4a131fec69e"
):
    fail(
        "bytecode-image-model-seal",
        "the atom-bearing bytecode image model drifted from its reviewed normalized implementation",
    )

image_atom_declaration = re.compile(
    r"\bpub[ \t\n]*\([ \t\n]*super[ \t\n]*\)[ \t\n]+enum"
    r"[ \t\n]+ImageAtom\b"
)
all_image_atom_declarations = re.compile(
    r"\bpub(?:[ \t\n]*\([^)]*\))?[ \t\n]+enum[ \t\n]+ImageAtom\b"
)
if (
    len(image_atom_declaration.findall(image_atoms_code)) != 1
    or len(all_image_atom_declarations.findall(image_atoms_code)) != 1
):
    fail(
        "image-atom-visibility",
        "ImageAtom must remain visible only to its bytecode_image parent module",
    )

eval_atom_constant = re.compile(
    r"(?m)^[ \t]*const[ \t]+PINNED_EVAL_ATOM_RAW[ \t]*:[ \t]*u32"
    r"[ \t]*=[ \t]*84[ \t]*;[ \t]*$"
)
if len(eval_atom_constant.findall(image_model_code)) != 1:
    fail(
        "scalar-script-atom-predicate",
        "the pinned <eval> identity must remain one private model constant with raw value 84",
    )

binary_object_visibility = (
    r"pub[ \t\n]*\([ \t\n]*in[ \t\n]+crate[ \t\n]*::[ \t\n]*runtime"
    r"[ \t\n]*::[ \t\n]*binary_object[ \t\n]*\)"
)
bytecode_image_impl_pattern = re.compile(
    r"\bimpl[ \t\n]+BytecodeImage[ \t\n]*\{"
)
model_bytecode_image_impl_code, _, _ = unique_braced_item(
    image_model_code,
    bytecode_image_impl_pattern,
    "bytecode-image-visible-method-set",
    "model-owned BytecodeImage implementation",
)
bytecode_image_impl_paths = []
for path, code in binary_code_cache.items():
    bytecode_image_impl_paths.extend(
        path.relative_to(root).as_posix()
        for _ in bytecode_image_impl_pattern.finditer(code)
    )
if bytecode_image_impl_paths != [image_model_relative]:
    fail(
        "bytecode-image-visible-method-set",
        "BytecodeImage implementations must remain in the reviewed model owner; "
        f"found {bytecode_image_impl_paths}",
    )
bytecode_image_visible_method_pattern = re.compile(
    r"\b(?P<visibility>pub(?:[ \t\n]*\([^)]*\))?)[ \t\n]+"
    r"(?:const[ \t\n]+)?fn[ \t\n]+(?P<name>[A-Za-z_][A-Za-z0-9_]*)"
)


def visible_method_set(code: str) -> list[tuple[str, str]]:
    return [
        (
            " ".join(match.group("visibility").split()),
            match.group("name"),
        )
        for match in bytecode_image_visible_method_pattern.finditer(code)
    ]


expected_model_bytecode_image_methods = [
    ("pub(super)", "new"),
    ("pub(in crate::runtime::binary_object)", "input_atom_slot_count"),
    ("pub(in crate::runtime)", "atoms"),
    ("pub(super)", "nodes"),
    ("pub(in crate::runtime::binary_object)", "sab_archive_occurrences"),
    ("pub(in crate::runtime)", "reference_table"),
    ("pub(in crate::runtime)", "functions"),
    ("pub(in crate::runtime)", "function"),
    ("pub(in crate::runtime)", "modules"),
    ("pub(in crate::runtime)", "module"),
    ("pub(in crate::runtime)", "root"),
]
model_bytecode_image_methods = visible_method_set(model_bytecode_image_impl_code)
if model_bytecode_image_methods not in (expected_model_bytecode_image_methods, []):
    fail(
        "bytecode-image-visible-method-set",
        "BytecodeImage may expose only its reviewed model accessors",
    )

null_name_predicate = re.compile(
    rf"\b{binary_object_visibility}[ \t\n]+const[ \t\n]+fn"
    r"[ \t\n]+name_is_null[ \t\n]*\([ \t\n]*&self[ \t\n]*\)"
    r"[ \t\n]*->[ \t\n]*bool[ \t\n]*\{[ \t\n]*matches!"
    r"[ \t\n]*\([ \t\n]*self[ \t\n]*\.[ \t\n]*name[ \t\n]*,"
    r"[ \t\n]*ImageAtom[ \t\n]*::[ \t\n]*Null[ \t\n]*\)"
    r"[ \t\n]*\}"
)
eval_name_predicate = re.compile(
    rf"\b{binary_object_visibility}[ \t\n]+const[ \t\n]+fn"
    r"[ \t\n]+name_is_pinned_eval[ \t\n]*\([ \t\n]*&self[ \t\n]*\)"
    r"[ \t\n]*->[ \t\n]*bool[ \t\n]*\{[ \t\n]*match"
    r"[ \t\n]+self[ \t\n]*\.[ \t\n]*name[ \t\n]*\{"
    r"[ \t\n]*ImageAtom[ \t\n]*::[ \t\n]*Predefined[ \t\n]*\("
    r"[ \t\n]*atom[ \t\n]*\)[ \t\n]*=>[ \t\n]*atom[ \t\n]*\."
    r"[ \t\n]*raw[ \t\n]*\([ \t\n]*\)[ \t\n]*==[ \t\n]*PINNED_EVAL_ATOM_RAW"
    r"[ \t\n]*,[ \t\n]*ImageAtom[ \t\n]*::[ \t\n]*Null[ \t\n]*\|"
    r"[ \t\n]*ImageAtom[ \t\n]*::[ \t\n]*Index[ \t\n]*\([ \t\n]*_[ \t\n]*\)"
    r"[ \t\n]*\|[ \t\n]*ImageAtom[ \t\n]*::[ \t\n]*Dynamic"
    r"[ \t\n]*\([ \t\n]*_[ \t\n]*\)[ \t\n]*=>[ \t\n]*false[ \t\n]*,?"
    r"[ \t\n]*\}[ \t\n]*\}"
)
if (
    len(null_name_predicate.findall(image_model_code)) != (2 if is_full_binary_inventory else 1)
    or len(eval_name_predicate.findall(image_model_code)) != 1
):
    fail(
        "scalar-script-atom-predicate",
        "the model must expose only the reviewed null-local, null-function-name, and pinned-<eval> boolean predicates",
    )

image_atom_export = re.compile(
    r"\bpub(?:[ \t\n]*\([^)]*\))?[ \t\n]+use\b[^;]*"
    r"\b(?:ImageAtom|PinnedAtomId)\b",
    re.DOTALL,
)
raw_atom_return = re.compile(
    rf"\b{binary_object_visibility}[^;{{}}]*\bfn\b[^;{{}}]*->[ \t\n]*"
    r"(?:ImageAtom|PinnedAtomId)\b"
)
visible_function_pattern = re.compile(
    r"\b(?P<visibility>pub(?:[ \t\n]*\([^)]*\))?)[ \t\n]+"
    r"(?:(?:const|async|unsafe|extern)[ \t\n]+)*fn[ \t\n]+"
    r"(?P<name>[A-Za-z_][A-Za-z0-9_]*)\b[^;{}]*\{"
)
atom_sensitive_visible_sites: list[tuple[str, str, str]] = []
for path in binary_sources:
    if path.is_symlink() or not path.is_file():
        continue
    relative = path.relative_to(root).as_posix()
    source = binary_source_cache[path]
    code = binary_code_cache[path]
    for match in image_atom_export.finditer(code):
        fail(
            "image-atom-reexport",
            "ImageAtom and PinnedAtomId must not cross the bytecode_image boundary; found "
            + location(relative, source, match.start()),
        )
    for match in raw_atom_return.finditer(code):
        fail(
            "image-atom-escape",
            "scalar admission may consume boolean atom predicates, not raw atom identities; found "
            + location(relative, source, match.start()),
        )
    if not relative.startswith("src/runtime/binary_object/bytecode_image/") or path.name == "tests.rs":
        continue
    for match in visible_function_pattern.finditer(code):
        visibility = " ".join(match.group("visibility").split())
        if visibility in ("pub(super)", "pub(self)"):
            continue
        item_code, _, _ = braced_item_from_match(
            code,
            match,
            "image-atom-visible-capability",
            "runtime-visible bytecode_image function",
        )
        if re.search(
            r"\b(?:ImageAtom|PinnedAtomId)\b|\.[ \t\n]*(?:atom|raw)[ \t\n]*\(",
            item_code,
        ):
            atom_sensitive_visible_sites.append(
                (relative, visibility, match.group("name"))
            )

expected_atom_sensitive_visible_sites = [
    (
        "src/runtime/binary_object/bytecode_image/model.rs",
        "pub(in crate::runtime::binary_object)",
        "name_is_null",
    ),
    (
        "src/runtime/binary_object/bytecode_image/model.rs",
        "pub(in crate::runtime::binary_object)",
        "name_is_pinned_eval",
    ),
]
if is_full_binary_inventory:
    expected_atom_sensitive_visible_sites.append(
        (
            "src/runtime/binary_object/bytecode_image/model.rs",
            "pub(in crate::runtime::binary_object)",
            "name_is_null",
        )
    )
if atom_sensitive_visible_sites != expected_atom_sensitive_visible_sites:
    fail(
        "image-atom-visible-capability",
        "only the reviewed boolean atom predicates may expose an atom-sensitive bytecode-image method; "
        f"found {atom_sensitive_visible_sites}",
    )

retired_permit_names = (
    "GraphSabDecodePermit",
    "BytecodeImageSabDecodePermit",
    "SabDecodePermit",
    "sab_decode_permit_sealed",
)
for path in binary_sources:
    if path.is_symlink() or not path.is_file():
        continue
    relative = path.relative_to(root).as_posix()
    source = binary_source_cache[path]
    code = binary_code_cache[path]
    for name in retired_permit_names:
        match = re.search(rf"\b{re.escape(name)}\b", code)
        if match is not None:
            fail(
                "retired-sab-permit",
                f"{name} must not reintroduce forgeable cross-module permits; found "
                + location(relative, source, match.start()),
            )

native_token_struct = re.compile(
    r"\bpub[ \t\n]*\([ \t\n]*in[ \t\n]+crate[ \t\n]*::[ \t\n]*runtime"
    r"[ \t\n]*\)[ \t\n]+struct[ \t\n]+NativeSabToken[ \t\n]*\{"
    r"[ \t\n]*native_token_bits[ \t\n]*:[ \t\n]*u64[ \t\n]*,?[ \t\n]*\}"
)
if len(native_token_struct.findall(sab_transport_code)) != 1:
    fail(
        "sab-native-token-shape",
        "NativeSabToken must retain one private named u64 field",
    )
if re.search(
    r"#[ \t\n]*\[[^\]]*\][ \t\n]*pub[ \t\n]*\([^)]*runtime[^)]*\)"
    r"[ \t\n]+struct[ \t\n]+NativeSabToken\b",
    sab_transport_code,
):
    fail(
        "sab-native-token-derive",
        "NativeSabToken must not gain derive or representation attributes",
    )
native_token_impl_pattern = re.compile(
    r"\bimpl\b[^{};]*\bNativeSabToken\b[^{};]*\{",
    re.DOTALL,
)
native_token_impl_sites: list[tuple[str, str, re.Match[str]]] = []
for path in binary_sources:
    if path.is_symlink() or not path.is_file():
        continue
    relative = path.relative_to(root).as_posix()
    code = binary_code_cache[path]
    native_token_impl_sites.extend(
        (relative, code, match) for match in native_token_impl_pattern.finditer(code)
    )
if len(native_token_impl_sites) != 1 or native_token_impl_sites[0][0] != sab_transport_relative:
    fail(
        "sab-native-token-implementation-set",
        "NativeSabToken must have one test-only transport-owned implementation",
    )
else:
    _, code, match = native_token_impl_sites[0]
    prefix = code[max(0, match.start() - 80):match.start()]
    if re.search(r"#\s*\[\s*cfg\s*\(\s*test\s*\)\s*\]\s*$", prefix) is None:
        fail(
            "sab-native-token-test-only",
            "NativeSabToken implementation must remain guarded by cfg(test)",
        )
    open_offset = match.end() - 1
    depth = 0
    close_offset = None
    for offset in range(open_offset, len(code)):
        character = code[offset]
        if character == "{":
            depth += 1
        elif character == "}":
            depth -= 1
            if depth == 0:
                close_offset = offset + 1
                break
    if close_offset is None:
        fail(
            "sab-native-token-implementation-set",
            "NativeSabToken implementation has no balanced closing brace",
        )
    else:
        actual_impl = " ".join(code[match.start():close_offset].split())
        expected_impl = " ".join(
            """
            impl NativeSabToken {
                #[must_use]
                pub(in crate::runtime::binary_object) const fn from_test_bits(bits: u64) -> Self {
                    Self {
                        native_token_bits: bits,
                    }
                }
            }
            """.split()
        )
        if actual_impl != expected_impl:
            fail(
                "sab-native-token-implementation-set",
                "NativeSabToken implementation drifted from its reviewed test-only constructor",
            )

def identifier_paths(name: str) -> list[str]:
    pattern = re.compile(rf"\b{re.escape(name)}\b")
    found: list[str] = []
    for path in binary_sources:
        if path.is_symlink() or not path.is_file():
            continue
        relative = path.relative_to(root).as_posix()
        code = binary_code_cache[path]
        found.extend(relative for _ in pattern.finditer(code))
    return found


native_token_field_paths = identifier_paths("native_token_bits")
if native_token_field_paths != [sab_transport_relative] * 3:
    fail(
        "sab-native-token-field-escape",
        "native_token_bits must appear only in its field, test constructor, and matcher; "
        f"found {native_token_field_paths}",
    )


def definition_paths(kind: str, name: str) -> list[str]:
    pattern = re.compile(rf"\b{kind}[ \t\n]+{re.escape(name)}\b")
    found: list[str] = []
    for path in binary_sources:
        if path.is_symlink() or not path.is_file():
            continue
        relative = path.relative_to(root).as_posix()
        code = binary_code_cache[path]
        found.extend(relative for _ in pattern.finditer(code))
    return found


owned_definitions = (
    ("fn", "build_cursor", sab_transport_relative),
    ("fn", "finish_shared_backings", sab_transport_relative),
    ("fn", "finish_graph_archive", sab_transport_relative),
    ("fn", "finish_bytecode_image", sab_transport_relative),
    ("fn", "decode_graph_with_sab_transport", sab_transport_relative),
    ("fn", "decode_bytecode_image_with_sab_transport", sab_transport_relative),
    ("struct", "ArchivedBytecodeImage", sab_transport_relative),
)
for kind, name, expected_path in owned_definitions:
    found = definition_paths(kind, name)
    if found != [expected_path]:
        fail(
            "sab-transport-owner",
            f"{kind} {name} must have one definition owned by {expected_path}; found {found}",
        )

private_member_patterns = (
    "build_cursor",
    "finish_shared_backings",
    "finish_graph_archive",
    "finish_bytecode_image",
)
for name in private_member_patterns:
    pattern = re.compile(rf"(?m)^[ \t]*fn[ \t]+{re.escape(name)}\b")
    if len(pattern.findall(sab_transport_code)) != 1:
        fail(
            "sab-transport-private-member",
            f"{name} must remain a single module-private SAB transport method",
        )

entrypoint_patterns = (
    "decode_graph_with_sab_transport",
    "decode_bytecode_image_with_sab_transport",
)
for name in entrypoint_patterns:
    pattern = re.compile(
        rf"\bpub[ \t\n]*\([ \t\n]*in[ \t\n]+crate[ \t\n]*::[ \t\n]*runtime"
        rf"[ \t\n]*\)[ \t\n]+fn[ \t\n]+{re.escape(name)}\b"
    )
    if len(pattern.findall(sab_transport_code)) != 1:
        fail(
            "sab-transport-entrypoint",
            f"{name} must remain one runtime-private complete-input entrypoint",
        )

body_visibility_specs = (
    (
        graph_decode_code,
        re.compile(
            r"\bpub[ \t\n]*\([ \t\n]*super[ \t\n]*\)[ \t\n]+fn"
            r"[ \t\n]+decode_graph_body\b"
        ),
        "decode_graph_body",
    ),
    (
        image_decode_code,
        re.compile(
            r"\bpub[ \t\n]*\([ \t\n]*in[ \t\n]+crate[ \t\n]*::[ \t\n]*runtime"
            r"[ \t\n]*::[ \t\n]*binary_object[ \t\n]*\)[ \t\n]+fn"
            r"[ \t\n]+decode_bytecode_image_body\b"
        ),
        "decode_bytecode_image_body",
    ),
)
for code, pattern, name in body_visibility_specs:
    if len(pattern.findall(code)) != 1:
        fail(
            "sab-decoder-body-visibility",
            f"{name} must retain its narrow transport-owner visibility",
        )

call_site_specs = (
    ("build_cursor", [sab_transport_relative] * 4),
    ("finish_shared_backings", [sab_transport_relative] * 3),
    ("finish_graph_archive", [sab_transport_relative] * 3),
    ("finish_bytecode_image", [sab_transport_relative] * 2),
    ("sab_archive_occurrences", [image_model_relative, sab_transport_relative]),
)
for name, expected_paths in call_site_specs:
    found = identifier_paths(name)
    if sorted(found) != sorted(expected_paths):
        fail(
            "sab-archive-call-site-set",
            f"{name} must remain confined to its canonical definition and binders; found {found}",
        )

cursor_struct = re.compile(
    r"\bpub[ \t\n]*\([ \t\n]*in[ \t\n]+crate[ \t\n]*::[ \t\n]*runtime"
    r"[ \t\n]*::[ \t\n]*binary_object[ \t\n]*\)[ \t\n]+struct"
    r"[ \t\n]+SabTransportCursor[ \t\n]*<[ \t\n]*'a[ \t\n]*>[ \t\n]*\{"
    r"[ \t\n]*cursor_wire[ \t\n]*:[ \t\n]*WireCursor[ \t\n]*<[ \t\n]*'a[ \t\n]*>[ \t\n]*,"
    r"[ \t\n]*cursor_writer_occurrences[ \t\n]*:[ \t\n]*&[ \t\n]*'a[ \t\n]*"
    r"\[[ \t\n]*NativeSabToken[ \t\n]*\][ \t\n]*,"
    r"[ \t\n]*cursor_next_occurrence[ \t\n]*:[ \t\n]*usize[ \t\n]*,"
    r"[ \t\n]*cursor_archive[ \t\n]*:[ \t\n]*SabArchiveState[ \t\n]*,?[ \t\n]*\}"
)
if len(cursor_struct.findall(sab_transport_code)) != 1:
    fail(
        "sab-cursor-shape",
        "SabTransportCursor must retain four private transport-owned fields",
    )

input_struct = re.compile(
    r"\bpub[ \t\n]*\([ \t\n]*in[ \t\n]+crate[ \t\n]*::[ \t\n]*runtime"
    r"[ \t\n]*\)[ \t\n]+struct[ \t\n]+SabTransportInput"
    r"[ \t\n]*<[ \t\n]*'a[ \t\n]*>[ \t\n]*\{"
    r"[ \t\n]*transport_wire_bytes[ \t\n]*:[ \t\n]*&[ \t\n]*'a"
    r"[ \t\n]*\[[ \t\n]*u8[ \t\n]*\][ \t\n]*,"
    r"[ \t\n]*transport_writer_occurrences[ \t\n]*:[ \t\n]*&[ \t\n]*'a"
    r"[ \t\n]*\[[ \t\n]*NativeSabToken[ \t\n]*\][ \t\n]*,?[ \t\n]*\}"
)
if len(input_struct.findall(sab_transport_code)) != 1:
    fail(
        "sab-input-shape",
        "SabTransportInput must retain two private transport-owned fields",
    )

input_impl_pattern = re.compile(
    r"\bimpl[ \t\n]*<[ \t\n]*'a[ \t\n]*>[ \t\n]*"
    r"SabTransportInput[ \t\n]*<[ \t\n]*'a[ \t\n]*>[ \t\n]*\{"
)
input_impl_matches = list(input_impl_pattern.finditer(sab_transport_code))
if len(input_impl_matches) != 1:
    fail(
        "sab-input-implementation-set",
        "SabTransportInput must have one reviewed transport-owned implementation",
    )
else:
    match = input_impl_matches[0]
    open_offset = match.end() - 1
    depth = 0
    close_offset = None
    for offset in range(open_offset, len(sab_transport_code)):
        character = sab_transport_code[offset]
        if character == "{":
            depth += 1
        elif character == "}":
            depth -= 1
            if depth == 0:
                close_offset = offset + 1
                break
    if close_offset is None:
        fail(
            "sab-input-implementation-set",
            "SabTransportInput implementation has no balanced closing brace",
        )
    else:
        actual_impl = " ".join(sab_transport_code[match.start():close_offset].split())
        expected_impl = " ".join(
            """
            impl<'a> SabTransportInput<'a> {
                #[must_use]
                pub(in crate::runtime) const fn new(
                    wire: &'a [u8],
                    writer_occurrences: &'a [NativeSabToken],
                ) -> Self {
                    Self {
                        transport_wire_bytes: wire,
                        transport_writer_occurrences: writer_occurrences,
                    }
                }
                fn build_cursor(
                    self,
                    mode: ReaderMode,
                    wire_limits: WireLimits,
                    graph_limits: GraphLimits,
                ) -> Result<SabTransportCursor<'a>, SabArchiveError> {
                    Ok(SabTransportCursor {
                        cursor_wire: WireCursor::new(self.transport_wire_bytes, mode, wire_limits)?,
                        cursor_writer_occurrences: self.transport_writer_occurrences,
                        cursor_next_occurrence: 0,
                        cursor_archive: SabArchiveState::new(graph_limits),
                    })
                }
                #[cfg(test)]
                fn into_cursor_for_test(
                    self,
                    mode: ReaderMode,
                    wire_limits: WireLimits,
                    graph_limits: GraphLimits,
                ) -> Result<SabTransportCursor<'a>, SabArchiveError> {
                    self.build_cursor(mode, wire_limits, graph_limits)
                }
            }
            """.split()
        )
        if actual_impl != expected_impl:
            fail(
                "sab-input-implementation-set",
                "SabTransportInput implementation drifted from its reviewed inseparable surface",
            )

def normalized_function(code: str, name: str) -> str | None:
    pattern = re.compile(
        rf"\bpub[ \t\n]*\([^)]*\)[ \t\n]+fn[ \t\n]+{re.escape(name)}\b"
    )
    matches = list(pattern.finditer(code))
    if len(matches) != 1:
        return None
    match = matches[0]
    open_offset = code.find("{", match.end())
    if open_offset < 0:
        return None
    depth = 0
    close_offset = None
    for offset in range(open_offset, len(code)):
        character = code[offset]
        if character == "{":
            depth += 1
        elif character == "}":
            depth -= 1
            if depth == 0:
                close_offset = offset + 1
                break
    if close_offset is None:
        return None
    return " ".join(code[match.start():close_offset].split())


reviewed_entrypoints = (
    (
        "decode_graph_with_sab_transport",
        """
        pub(in crate::runtime) fn decode_graph_with_sab_transport(
            input: SabTransportInput<'_>,
            mode: ReaderMode,
            wire_limits: WireLimits,
            graph_limits: GraphLimits,
            allow_object_references: bool,
        ) -> Result<ArchivedWireGraph, DecodeError> {
            let cursor = input
                .build_cursor(mode, wire_limits, graph_limits)
                .map_err(map_sab_archive_error)?;
            let (cursor, graph) =
                decode_graph_body(cursor, graph_limits, allow_object_references)?;
            cursor
                .finish_graph_archive(graph)
                .map_err(map_sab_archive_error)
        }
        """,
    ),
    (
        "decode_bytecode_image_with_sab_transport",
        """
        pub(in crate::runtime) fn decode_bytecode_image_with_sab_transport(
            input: SabTransportInput<'_>,
            mode: ReaderMode,
            wire_limits: WireLimits,
            limits: BytecodeImageLimits,
            allow_object_references: bool,
        ) -> Result<ArchivedBytecodeImage, BytecodeImageError> {
            let cursor = input.build_cursor(mode, wire_limits, limits.graph())?;
            let (cursor, image) =
                decode_bytecode_image_body(cursor, limits, allow_object_references)?;
            cursor.finish_bytecode_image(image).map_err(Into::into)
        }
        """,
    ),
)
for name, expected_source in reviewed_entrypoints:
    actual = normalized_function(sab_transport_code, name)
    expected = " ".join(expected_source.split())
    if actual != expected:
        fail(
            "sab-transport-entrypoint-body",
            f"{name} drifted from its reviewed complete-input decode and finalization path",
        )

transport_field_counts = (
    ("transport_wire_bytes", 3),
    ("transport_writer_occurrences", 3),
    ("cursor_wire", 18),
    ("cursor_writer_occurrences", 6),
    ("cursor_next_occurrence", 6),
    ("cursor_archive", 4),
)
for name, expected in transport_field_counts:
    found = identifier_paths(name)
    if found != [sab_transport_relative] * expected:
        fail(
            "sab-transport-field-escape",
            f"{name} must appear only in its reviewed transport-owned operations; found {found}",
        )

cursor_alias = re.compile(
    r"\bSabTransportCursor[ \t\n]+as[ \t\n]+"
    r"|\btype[ \t\n]+(?:r#)?[A-Za-z_][A-Za-z0-9_]*[ \t\n]*"
    r"(?:<[^;=]*>)?[ \t\n]*=[^;]*\bSabTransportCursor\b"
)
for match in cursor_alias.finditer(sab_transport_code):
    fail(
        "sab-cursor-alias",
        "the transport owner must not alias its cursor around construction gates; found "
        + location(sab_transport_relative, sab_transport_source, match.start()),
    )

cursor_impl_pattern = re.compile(
    r"\bimpl\b[^{};]*\bSabTransportCursor\b[^{};]*\{",
    re.DOTALL,
)
# Capture the complete token after fn, not merely an ASCII Rust-name
# subset. Rust accepts Unicode XID identifiers, and an unknown valid name
# must make every reviewed function or method surface fail closed.
function_name_token = re.compile(r"\bfn[ \t\n]+((?:r#)?[^\s(<{]+)")
cursor_impl_matches = list(cursor_impl_pattern.finditer(sab_transport_code))
cursor_impl_headers = [
    " ".join(sab_transport_code[match.start():match.end()].split())
    for match in cursor_impl_matches
]
if cursor_impl_headers != ["impl<'a> SabTransportCursor<'a> {"]:
    fail(
        "sab-cursor-implementation-set",
        "SabTransportCursor must have one transport-owned inherent implementation; "
        f"found {cursor_impl_headers}",
    )
else:
    match = cursor_impl_matches[0]
    open_offset = match.end() - 1
    depth = 0
    close_offset = None
    for offset in range(open_offset, len(sab_transport_code)):
        character = sab_transport_code[offset]
        if character == "{":
            depth += 1
        elif character == "}":
            depth -= 1
            if depth == 0:
                close_offset = offset + 1
                break
    if close_offset is None:
        fail(
            "sab-cursor-implementation-set",
            "SabTransportCursor implementation has no balanced closing brace",
        )
        cursor_impl_code = ""
    else:
        cursor_impl_code = sab_transport_code[open_offset:close_offset]

    cursor_methods: list[str] = []
    for method in function_name_token.finditer(cursor_impl_code):
        prefix = cursor_impl_code[:method.start()]
        if prefix.count("{") - prefix.count("}") == 1:
            cursor_methods.append(method.group(1))
    expected_cursor_methods = [
        "position",
        "mode",
        "remaining",
        "read_u8",
        "read_u16_le",
        "read_bytes",
        "read_tag",
        "read_uleb128",
        "read_i32",
        "read_f64",
        "read_header",
        "read_string",
        "validate_wire_end",
        "record_shared_array_buffer",
        "finish_shared_backings",
        "finish_graph_archive",
        "finish_graph_archive_for_test",
        "finish_bytecode_image",
    ]
    if cursor_methods != expected_cursor_methods:
        fail(
            "sab-cursor-method-set",
            "SabTransportCursor method surface drifted; "
            f"found {cursor_methods}",
        )

    binary_object_cursor_methods = expected_cursor_methods[:13]
    for name in binary_object_cursor_methods:
        pattern = re.compile(
            rf"(?m)^[ \t]*pub[ \t]*\([ \t]*in[ \t]+crate::runtime::binary_object"
            rf"[ \t]*\)[ \t]+(?:const[ \t]+)?fn[ \t]+{re.escape(name)}\b"
        )
        if len(pattern.findall(cursor_impl_code)) != 1:
            fail(
                "sab-cursor-method-visibility",
                f"SabTransportCursor::{name} must remain binary_object-private",
            )
    if len(
        re.findall(
            r"(?m)^[ \t]*pub[ \t]*\([ \t]*super[ \t]*\)[ \t]+fn"
            r"[ \t]+record_shared_array_buffer\b",
            cursor_impl_code,
        )
    ) != 1:
        fail(
            "sab-cursor-method-visibility",
            "record_shared_array_buffer must remain graph-private",
        )
    for name in (
        "finish_shared_backings",
        "finish_graph_archive",
        "finish_graph_archive_for_test",
        "finish_bytecode_image",
    ):
        pattern = re.compile(rf"(?m)^[ \t]*fn[ \t]+{re.escape(name)}\b")
        if len(pattern.findall(cursor_impl_code)) != 1:
            fail(
                "sab-cursor-method-visibility",
                f"SabTransportCursor::{name} must remain module-private",
            )

    test_graph_finalizer = re.compile(
        r"#\[cfg\(test\)\][ \t\n]*fn[ \t\n]+finish_graph_archive_for_test"
        r"[ \t\n]*\([ \t\n]*self[ \t\n]*,[ \t\n]*graph[ \t\n]*:"
        r"[ \t\n]*WireGraph[ \t\n]*,[ \t\n]*\)[ \t\n]*->"
        r"[ \t\n]*Result[ \t\n]*<[ \t\n]*ArchivedWireGraph[ \t\n]*,"
        r"[ \t\n]*SabArchiveError[ \t\n]*>[ \t\n]*\{"
        r"[ \t\n]*self[ \t\n]*\.[ \t\n]*finish_graph_archive"
        r"[ \t\n]*\([ \t\n]*graph[ \t\n]*\)[ \t\n]*\}"
    )
    if len(test_graph_finalizer.findall(cursor_impl_code)) != 1:
        fail(
            "sab-test-finalizer-body",
            "the graph-finalizer test shim must remain cfg(test) and delegate exactly once",
        )

top_level_functions: list[str] = []
for match in function_name_token.finditer(sab_transport_code):
    prefix = sab_transport_code[:match.start()]
    if prefix.count("{") == prefix.count("}"):
        top_level_functions.append(match.group(1))
if top_level_functions != [
    "decode_graph_with_sab_transport",
    "decode_bytecode_image_with_sab_transport",
]:
    fail(
        "sab-transport-free-function-set",
        "SAB transport owner must expose only the two reviewed complete-input free functions; "
        f"found {top_level_functions}",
    )

# A const/static callable, type-level callable, trait, union, or foreign item
# can bypass a function-only surface even though it is not needed by this
# owner. Reject those top-level item classes outright, and reject re-exports;
# the reviewed structs/enums/inherent impls are locked independently below.
forbidden_top_level_items: list[str] = []
item_keyword = re.compile(r"\b(union|trait|type|const|static|extern)\b")
for match in item_keyword.finditer(sab_transport_code):
    prefix = sab_transport_code[:match.start()]
    if prefix.count("{") == prefix.count("}"):
        forbidden_top_level_items.append(match.group(1))
if forbidden_top_level_items:
    fail(
        "sab-transport-top-level-item-set",
        "SAB transport owner gained a forbidden top-level item; "
        f"found {forbidden_top_level_items}",
    )
owner_public_use = re.compile(
    r"\bpub(?:[ \t\n]*\([^)]*\))?[ \t\n]+use\b"
)
for match in owner_public_use.finditer(sab_transport_code):
    prefix = sab_transport_code[:match.start()]
    if prefix.count("{") == prefix.count("}"):
        fail(
            "sab-transport-top-level-item-set",
            "SAB transport owner must not re-export transport internals",
        )

graph_archive_struct = re.compile(
    r"\bpub[ \t\n]*\([ \t\n]*in[ \t\n]+crate[ \t\n]*::[ \t\n]*runtime"
    r"[ \t\n]*\)[ \t\n]+struct[ \t\n]+ArchivedWireGraph[ \t\n]*\{"
    r"[ \t\n]*archived_graph_payload[ \t\n]*:[ \t\n]*WireGraph[ \t\n]*,"
    r"[ \t\n]*archived_graph_shared_backings[ \t\n]*:[ \t\n]*Box[ \t\n]*<"
    r"[ \t\n]*\[[ \t\n]*SharedBackingDescriptor[ \t\n]*\][ \t\n]*>"
    r"[ \t\n]*,?[ \t\n]*\}"
)
if len(graph_archive_struct.findall(sab_transport_code)) != 1:
    fail(
        "sab-graph-aggregate-shape",
        "ArchivedWireGraph must retain exactly two private transport-owned fields",
    )

graph_archive_brace_paths: list[str] = []
graph_archive_brace_pattern = re.compile(r"\bArchivedWireGraph[ \t\n]*\{")
for path in binary_sources:
    if path.is_symlink() or not path.is_file():
        continue
    relative = path.relative_to(root).as_posix()
    code = binary_code_cache[path]
    graph_archive_brace_paths.extend(
        relative for _ in graph_archive_brace_pattern.finditer(code)
    )
if graph_archive_brace_paths != [sab_transport_relative] * 3:
    fail(
        "sab-graph-construction-set",
        "ArchivedWireGraph must have one declaration, literal, and reviewed implementation; "
        f"found {graph_archive_brace_paths}",
    )

graph_archive_field_counts = (
    ("archived_graph_payload", 3),
    ("archived_graph_shared_backings", 4),
)
for name, expected in graph_archive_field_counts:
    found = identifier_paths(name)
    if found != [sab_transport_relative] * expected:
        fail(
            "sab-graph-field-escape",
            f"{name} must appear only in the reviewed field, binder, and test projection; "
            f"found {found}",
        )

graph_archive_impl_pattern = re.compile(
    r"\bimpl\b[^{};]*\bArchivedWireGraph\b[^{};]*\{",
    re.DOTALL,
)
graph_archive_impl_sites: list[tuple[str, str, re.Match[str]]] = []
for path in binary_sources:
    if path.is_symlink() or not path.is_file():
        continue
    relative = path.relative_to(root).as_posix()
    code = binary_code_cache[path]
    graph_archive_impl_sites.extend(
        (relative, code, match) for match in graph_archive_impl_pattern.finditer(code)
    )
if len(graph_archive_impl_sites) != 1 or graph_archive_impl_sites[0][0] != sab_transport_relative:
    fail(
        "sab-graph-aggregate-escape",
        "ArchivedWireGraph must have one reviewed transport-owned inherent implementation",
    )
else:
    _, code, match = graph_archive_impl_sites[0]
    open_offset = match.end() - 1
    depth = 0
    close_offset = None
    for offset in range(open_offset, len(code)):
        character = code[offset]
        if character == "{":
            depth += 1
        elif character == "}":
            depth -= 1
            if depth == 0:
                close_offset = offset + 1
                break
    if close_offset is None:
        fail(
            "sab-graph-aggregate-escape",
            "ArchivedWireGraph implementation has no balanced closing brace",
        )
    else:
        actual_impl = " ".join(code[match.start():close_offset].split())
        expected_impl = " ".join(
            """
            impl ArchivedWireGraph {
                #[must_use]
                pub(in crate::runtime::binary_object) const fn shared_backing_count(&self) -> usize {
                    self.archived_graph_shared_backings.len()
                }
                #[cfg(test)]
                pub(in crate::runtime::binary_object) const fn test_graph(&self) -> &WireGraph {
                    &self.archived_graph_payload
                }
                #[cfg(test)]
                pub(super) fn test_shared_backing_descriptor(
                    &self,
                    backing: ArchiveBackingId,
                ) -> Option<SharedBackingDescriptor> {
                    self.archived_graph_shared_backings
                        .get(backing.as_usize())
                        .copied()
                }
            }
            """.split()
        )
        if actual_impl != expected_impl:
            fail(
                "sab-graph-aggregate-escape",
                "ArchivedWireGraph implementation drifted from its reviewed non-splitting surface",
            )

archive_struct = re.compile(
    r"\bpub[ \t\n]*\([ \t\n]*in[ \t\n]+crate[ \t\n]*::[ \t\n]*runtime"
    r"[ \t\n]*\)[ \t\n]+struct[ \t\n]+ArchivedBytecodeImage[ \t\n]*\{"
    r"[ \t\n]*archived_image_payload[ \t\n]*:[ \t\n]*BytecodeImage[ \t\n]*,"
    r"[ \t\n]*archived_image_shared_backings[ \t\n]*:[ \t\n]*Box[ \t\n]*<"
    r"[ \t\n]*\[[ \t\n]*SharedBackingDescriptor[ \t\n]*\][ \t\n]*>"
    r"[ \t\n]*,?[ \t\n]*\}"
)
if len(archive_struct.findall(sab_transport_code)) != 1:
    fail(
        "sab-image-aggregate-shape",
        "ArchivedBytecodeImage must retain exactly two private transport-owned fields",
    )

archive_brace_paths: list[str] = []
archive_brace_pattern = re.compile(r"\bArchivedBytecodeImage[ \t\n]*\{")
for path in binary_sources:
    if path.is_symlink() or not path.is_file():
        continue
    relative = path.relative_to(root).as_posix()
    code = binary_code_cache[path]
    archive_brace_paths.extend(relative for _ in archive_brace_pattern.finditer(code))
if archive_brace_paths != [sab_transport_relative] * 3:
    fail(
        "sab-image-construction-set",
        "ArchivedBytecodeImage must have one declaration, literal, and reviewed implementation; "
        f"found {archive_brace_paths}",
    )

archive_field_counts = (
    ("archived_image_payload", 3),
    ("archived_image_shared_backings", 4),
)
for name, expected in archive_field_counts:
    found = identifier_paths(name)
    if found != [sab_transport_relative] * expected:
        fail(
            "sab-image-field-escape",
            f"{name} must appear only in the reviewed field, binder, and test projection; "
            f"found {found}",
        )

archive_impl_pattern = re.compile(
    r"\bimpl\b[^{};]*\bArchivedBytecodeImage\b[^{};]*\{",
    re.DOTALL,
)
archive_impl_sites: list[tuple[str, str, re.Match[str]]] = []
for path in binary_sources:
    if path.is_symlink() or not path.is_file():
        continue
    relative = path.relative_to(root).as_posix()
    code = binary_code_cache[path]
    archive_impl_sites.extend((relative, code, match) for match in archive_impl_pattern.finditer(code))
if len(archive_impl_sites) != 1 or archive_impl_sites[0][0] != sab_transport_relative:
    fail(
        "sab-image-aggregate-escape",
        "ArchivedBytecodeImage must have one reviewed transport-owned inherent implementation",
    )
else:
    _, code, match = archive_impl_sites[0]
    open_offset = match.end() - 1
    depth = 0
    close_offset = None
    for offset in range(open_offset, len(code)):
        character = code[offset]
        if character == "{":
            depth += 1
        elif character == "}":
            depth -= 1
            if depth == 0:
                close_offset = offset + 1
                break
    if close_offset is None:
        fail(
            "sab-image-aggregate-escape",
            "ArchivedBytecodeImage implementation has no balanced closing brace",
        )
    else:
        actual_impl = " ".join(code[match.start():close_offset].split())
        expected_impl = " ".join(
            """
            impl ArchivedBytecodeImage {
                #[must_use]
                pub(in crate::runtime::binary_object) const fn shared_backing_count(&self) -> usize {
                    self.archived_image_shared_backings.len()
                }
                #[cfg(test)]
                pub(in crate::runtime::binary_object) const fn test_image(&self) -> &BytecodeImage {
                    &self.archived_image_payload
                }
                #[cfg(test)]
                pub(in crate::runtime::binary_object) fn test_shared_backing_descriptor(
                    &self,
                    backing: ArchiveBackingId,
                ) -> Option<SharedBackingDescriptor> {
                    self.archived_image_shared_backings
                        .get(backing.as_usize())
                        .copied()
                }
            }
            """.split()
        )
        if actual_impl != expected_impl:
            fail(
                "sab-image-aggregate-escape",
                "ArchivedBytecodeImage implementation drifted from its reviewed non-splitting surface",
            )

for path in binary_sources:
    relative = path.relative_to(root).as_posix()
    if path.is_symlink() or not path.is_file():
        fail("linked-source", f"{relative} must be a regular file")
        continue
    source = binary_source_cache[path]
    code = binary_code_cache[path]
    forbidden_patterns = (
        (
            "forbidden-vm-dependency",
            re.compile(r"\bcrate[ \t\n]*::[ \t\n]*(?:r#)?vm\b"),
            "crate::vm",
        ),
        (
            "forbidden-compiler-dependency",
            re.compile(r"\bcrate[ \t\n]*::[ \t\n]*(?:r#)?compiler\b"),
            "crate::compiler",
        ),
        (
            "forbidden-heap-dependency",
            re.compile(r"\bcrate[ \t\n]*::[ \t\n]*(?:r#)?heap\b"),
            "crate::heap",
        ),
        (
            "forbidden-runtime-dependency",
            re.compile(
                r"\buse[ \t\n]+crate[ \t\n]*::[ \t\n]*(?:r#)?runtime\b"
                r"(?![ \t\n]*::[ \t\n]*binary_object\b)"
            ),
            "crate::runtime",
        ),
        (
            "forbidden-shared-memory-dependency",
            re.compile(r"\bcrate[ \t\n]*::[ \t\n]*(?:r#)?shared_memory\b"),
            "crate::shared_memory",
        ),
        (
            "forbidden-parent-dependency",
            re.compile(
                r"\b(?:super[ \t\n]*::[ \t\n]*)+(?:r#)?(?:vm|compiler|heap)\b"
            ),
            "a parent-relative VM/compiler/heap path",
        ),
        (
            "forbidden-shared-memory-dependency",
            re.compile(
                r"\b(?:super[ \t\n]*::[ \t\n]*)+(?:r#)?shared_memory\b"
            ),
            "a parent-relative shared_memory path",
        ),
        (
            "forbidden-shared-memory-runtime-type",
            re.compile(r"\b(?:SharedBufferHandle|SharedBackingStore)\b"),
            "SharedBufferHandle or SharedBackingStore",
        ),
        (
            "forbidden-unsafe-code",
            re.compile(r"\bunsafe\b"),
            "unsafe Rust",
        ),
        (
            "forbidden-non-null-pointer",
            re.compile(r"\bNonNull\b"),
            "NonNull",
        ),
        (
            "forbidden-raw-pointer-type",
            re.compile(r"\*[ \t\n]*(?:const|mut)\b"),
            "a raw pointer type",
        ),
        (
            "forbidden-native-pointer-bridge",
            re.compile(
                r"\b(?:(?:[A-Za-z_][A-Za-z0-9_]*_)?from_raw_parts(?:_mut|_in)?"
                r"|into_raw(?:_with_allocator|_parts(?:_with_alloc)?)?)\b"
            ),
            "a native pointer ownership or slice bridge",
        ),
        (
            "forbidden-native-pointer-bridge",
            re.compile(
                r"\b(?:Box|Vec|Arc|Rc|CString|CStr)"
                r"[ \t\n]*(?:::[ \t\n]*<[^>{};\n]+>)?"
                r"[ \t\n]*::[ \t\n]*from_raw(?:_in)?\b"
            ),
            "a native pointer ownership bridge",
        ),
        (
            "forbidden-bytecode-function",
            re.compile(r"\b(?:BytecodeFunction|FunctionBytecodeRef)\b"),
            "BytecodeFunction or FunctionBytecodeRef",
        ),
        (
            "forbidden-runtime-representation",
            re.compile(
                r"\b(?:Runtime|Context|RuntimeError|FunctionMetadata|FunctionBytecodeData|"
                r"BytecodeConstant|FunctionBytecodeId|ObjectId|RawValue|UnlinkedFunction|"
                r"Instruction|Heap|HeapObject|ObjectRef)\b"
            ),
            "a runtime, verifier-draft, VM, or heap representation type",
        ),
        (
            "forbidden-publication-boundary",
            re.compile(
                r"\b(?:publish_unlinked_function|publish_verified_unlinked_function)\b"
            ),
            "a runtime publication function",
        ),
        (
            "forbidden-crate-alias",
            re.compile(
                r"\b(?:use[ \t\n]+crate[ \t\n]+as|extern[ \t\n]+crate[ \t\n]+self[ \t\n]+as)\b"
            ),
            "an alias for the crate root",
        ),
    )
    for code_name, pattern, description in forbidden_patterns:
        for match in pattern.finditer(code):
            fail(
                code_name,
                f"binary_object production sources must not depend on {description}; found "
                + location(relative, source, match.start()),
            )

    grouped_import_pattern = re.compile(
        r"\buse[ \t\n]+crate[ \t\n]*::[ \t\n]*\{(?P<body>.*?)\}[ \t\n]*;",
        re.DOTALL,
    )
    for grouped in grouped_import_pattern.finditer(code):
        body = grouped.group("body")
        if re.search(r"\b(?:r#)?vm\b", body):
            fail(
                "forbidden-vm-dependency",
                "binary_object production sources must not import crate::vm through a grouped use; found "
                + location(relative, source, grouped.start()),
            )
        if re.search(r"\b(?:r#)?compiler\b", body):
            fail(
                "forbidden-compiler-dependency",
                "binary_object production sources must not import crate::compiler through a grouped use; found "
                + location(relative, source, grouped.start()),
            )
        if re.search(r"\b(?:r#)?heap\b", body):
            fail(
                "forbidden-heap-dependency",
                "binary_object production sources must not import crate::heap through a grouped use; found "
                + location(relative, source, grouped.start()),
            )
        if re.search(r"\b(?:r#)?runtime\b", body):
            fail(
                "forbidden-runtime-dependency",
                "binary_object production sources must not import crate::runtime through a grouped use; found "
                + location(relative, source, grouped.start()),
            )
        if re.search(r"(?:^|[,{}])[ \t\n]*(?:r#)?shared_memory\b", body):
            fail(
                "forbidden-shared-memory-dependency",
                "binary_object production sources must not import crate::shared_memory through a grouped use; found "
                + location(relative, source, grouped.start()),
            )
        if re.search(r"\bself[ \t\n]+as\b", body):
            fail(
                "forbidden-crate-alias",
                "binary_object production sources must not alias the crate root through a grouped use; found "
                + location(relative, source, grouped.start()),
            )

    parent_grouped_import_pattern = re.compile(
        r"\buse[ \t\n]+(?:super[ \t\n]*::[ \t\n]*)+"
        r"\{(?P<body>.*?)\}[ \t\n]*;",
        re.DOTALL,
    )
    for grouped in parent_grouped_import_pattern.finditer(code):
        grouped_body = grouped.group("body")
        if re.search(r"(?:^|[,{}])[ \t\n]*(?:r#)?shared_memory\b", grouped_body):
            fail(
                "forbidden-shared-memory-dependency",
                "binary_object production sources must not import shared_memory through a parent-relative grouped use; found "
                + location(relative, source, grouped.start()),
            )
        if re.search(r"(?:^|[,{}])[ \t\n]*(?:r#)?heap\b", grouped_body):
            fail(
                "forbidden-heap-dependency",
                "binary_object production sources must not import heap through a parent-relative grouped use; found "
                + location(relative, source, grouped.start()),
            )

if errors:
    for error in errors:
        print(f"error: {error}", file=sys.stderr)
    raise SystemExit(1)
PY
}

run_stage3i_receipt_escape_canaries() {
    local suite_root=$1
    local base_root=$suite_root/base

    mkdir -p "$base_root/tests/fixtures" "$base_root/dev-support/test262" \
        "$base_root/docs"
    cp -R "$repository_root/src" "$base_root/src"
    cp -- "$repository_root/Cargo.toml" "$base_root/Cargo.toml"
    cp -- "$repository_root/tests/fixtures/function_bytecode_wire.c" \
        "$base_root/tests/fixtures/function_bytecode_wire.c"
    cp -- "$repository_root/tests/fixtures/function_bytecode_wire.quickjs-2026-06-04.txt" \
        "$base_root/tests/fixtures/function_bytecode_wire.quickjs-2026-06-04.txt"
    cp -- "$repository_root/dev-support/quickjs-c-oracles.tsv" \
        "$base_root/dev-support/quickjs-c-oracles.tsv"
    cp -- "$repository_root/dev-support/test262/current.conf" \
        "$base_root/dev-support/test262/current.conf"
    cp -- "$repository_root/docs/status.md" "$base_root/docs/status.md"
    cp -- "$repository_root/tests/test262-class-private-callables-b-global-candidate.tsv" \
        "$base_root/tests/test262-class-private-callables-b-global-candidate.tsv"
    cp -- "$repository_root/tests/test262-class-private-callables-b-global-candidate.jsonl" \
        "$base_root/tests/test262-class-private-callables-b-global-candidate.jsonl"

    expect_stage3i_receipt_multi_rewrite_rejected() {
        local label=$1
        local diagnostic=$2
        local plan=$3
        local field=${4-}
        local case_root=$suite_root/$label
        local output=$suite_root/$label.output

        cp -R "$base_root" "$case_root"
        python3 - "$case_root" "$plan" "$field" <<'PY'
from pathlib import Path
import hashlib
import os
import sys


root = Path(sys.argv[1])
plan = sys.argv[2]
field = sys.argv[3]
config_path = root / "dev-support/test262/current.conf"
status_path = root / "docs/status.md"
tsv_path = root / "tests/test262-class-private-callables-b-global-candidate.tsv"
jsonl_path = root / "tests/test262-class-private-callables-b-global-candidate.jsonl"


def replace_once(path: Path, before: str, after: str) -> None:
    source = path.read_text(encoding="utf-8")
    if source.count(before) != 1:
        raise SystemExit(
            f"Stage3I receipt canary expected one occurrence of {before!r} in {path}"
        )
    path.write_text(source.replace(before, after), encoding="utf-8")


if plan == "coherent-config-docs":
    config = config_path.read_text(encoding="utf-8")
    prefix = f"{field}="
    matches = [line for line in config.splitlines() if line.startswith(prefix)]
    if len(matches) != 1:
        raise SystemExit(f"Stage3I receipt canary requires exactly one {field} field")
    current = matches[0][len(prefix):]
    tampered = ("1" if current.startswith("0") else "0") + current[1:]
    replace_once(config_path, f"{field}={current}", f"{field}={tampered}")
    replace_once(status_path, current, tampered)
elif plan == "focused-content":
    replace_once(tsv_path, "# quickjs=2026-06-04", "# quickjs=2026-06-05")
elif plan == "self-consistent-four-file-forgery":
    replace_once(tsv_path, "# quickjs=2026-06-04", "# quickjs=2026-06-05")
    replace_once(
        jsonl_path,
        '\"quickjs\":\"2026-06-04\"',
        '\"quickjs\":\"2026-06-05\"',
    )
    config = config_path.read_text(encoding="utf-8")
    for key, receipt_path in (
        ("focused_tsv_sha256", tsv_path),
        ("focused_jsonl_sha256", jsonl_path),
    ):
        prefix = f"{key}="
        matches = [line for line in config.splitlines() if line.startswith(prefix)]
        if len(matches) != 1:
            raise SystemExit(f"Stage3I receipt canary requires exactly one {key} field")
        current = matches[0][len(prefix):]
        forged = hashlib.sha256(receipt_path.read_bytes()).hexdigest()
        replace_once(config_path, f"{key}={current}", f"{key}={forged}")
        replace_once(status_path, current, forged)
        config = config_path.read_text(encoding="utf-8")
elif plan == "status-html-wrapper":
    receipt_start = "The latest full R3fj execution"
    receipt_end = "focused or full metric reported above."
    replace_once(status_path, receipt_start, f"{field}\n\n{receipt_start}")
    replace_once(status_path, receipt_end, f"{receipt_end}\n\n</div>")
elif plan == "focused-hardlink":
    alias_path = root / "tests/stage3i-focused-receipt-hardlink.tsv"
    if alias_path.exists():
        raise SystemExit(f"Stage3I hardlink canary alias already exists: {alias_path}")
    os.link(tsv_path, alias_path)
else:
    raise SystemExit(f"unknown Stage3I receipt canary plan: {plan}")
PY
        if "$script_dir/check-binary-object-boundary.sh" --scan-only "$case_root" \
            > "$output" 2>&1; then
            die "Stage3I receipt escape canary escaped: $label"
        fi
        if [[ $(<"$output") != *"$diagnostic"* ]]; then
            echo "error: Stage3I receipt escape canary failed for the wrong reason: $label" >&2
            cat "$output" >&2
            exit 1
        fi
    }

    local label field
    while IFS=: read -r label field; do
        expect_stage3i_receipt_multi_rewrite_rejected \
            "$label" stage3i-receipt-pin coherent-config-docs "$field"
    done <<'STAGE3I_COHERENT_RECEIPT_CANARIES'
stage3i-receipt-coherent-source:engine_semantics_source
stage3i-receipt-coherent-fingerprint:engine_semantics_sha256
stage3i-receipt-coherent-focused-tsv:focused_tsv_sha256
stage3i-receipt-coherent-focused-jsonl:focused_jsonl_sha256
stage3i-receipt-coherent-full-tsv:full_tsv_sha256
stage3i-receipt-coherent-full-jsonl:full_jsonl_sha256
STAGE3I_COHERENT_RECEIPT_CANARIES
    expect_stage3i_receipt_multi_rewrite_rejected \
        stage3i-receipt-focused-content stage3i-focused-receipt focused-content
    expect_stage3i_receipt_multi_rewrite_rejected \
        stage3i-receipt-self-consistent-four-file-forgery stage3i-receipt-pin \
        self-consistent-four-file-forgery
    local wrapper
    for wrapper in \
        '<div style="display/**/:none">' \
        '<div style="display&#58;none">' \
        '<div style="visibility:collapse">'
    do
        label=${wrapper//[^A-Za-z0-9]/-}
        expect_stage3i_receipt_multi_rewrite_rejected \
            "stage3i-status-outside-wrapper-$label" stage3i-status \
            status-html-wrapper "$wrapper"
    done
    expect_stage3i_receipt_multi_rewrite_rejected \
        stage3i-receipt-true-hardlink stage3i-focused-receipt focused-hardlink
}

case ${1:-} in
    "") ;;
    --scan-only)
        [[ $# == 2 ]] || die "usage: $0 --scan-only ROOT"
        scan_root "$2"
        exit 0
        ;;
    --stage3i-receipt-canaries)
        [[ $# == 1 ]] \
            || die "usage: $0 --stage3i-receipt-canaries"
        scan_root "$repository_root"
        receipt_canary_tmp=$(mktemp -d \
            "${TMPDIR:-/tmp}/quickjs-oxide-stage3i-receipts.XXXXXX")
        trap 'rm -rf -- "$receipt_canary_tmp"' EXIT HUP INT TERM
        run_stage3i_receipt_escape_canaries "$receipt_canary_tmp"
        echo "Stage3I receipt escape canaries passed: 12/12 rejected"
        exit 0
        ;;
    *) die "usage: $0 [--scan-only ROOT|--stage3i-receipt-canaries]" ;;
esac

scan_root "$repository_root"

python3 - "$repository_root" <<'PY'
from pathlib import Path
import sys


root = Path(sys.argv[1])
invocation = "./scripts/check-binary-object-boundary.sh"

parity = (root / "scripts/test-parity-slice.sh").read_text(encoding="utf-8")
if parity.count(invocation) != 1:
    raise SystemExit(
        "error: scripts/test-parity-slice.sh must invoke the binary-object boundary exactly once"
    )

workflow = (root / ".github/workflows/ci.yml").read_text(encoding="utf-8")
try:
    fast = workflow.split("\n  fast:\n", 1)[1].split("\n  quickjs-differential:\n", 1)[0]
except IndexError as error:
    raise SystemExit("error: could not locate the public fast CI job") from error
if fast.count(invocation) != 1:
    raise SystemExit(
        "error: public fast CI must invoke the binary-object boundary exactly once"
    )
PY

tmp_dir=$(mktemp -d "${TMPDIR:-/tmp}/quickjs-oxide-binary-boundary.XXXXXX")
trap 'rm -rf -- "$tmp_dir"' EXIT HUP INT TERM

fixture=$tmp_dir/fixture
mkdir -p "$fixture/src/runtime/binary_object/bytecode_image/decode" \
    "$fixture/src/runtime/binary_object/function_translate" \
    "$fixture/src/runtime/binary_object/graph"
printf '%s\n' 'generated boundary self-test fixture' > "$fixture/.boundary-self-test"
printf '%s\n' 'pub mod runtime;' > "$fixture/src/lib.rs"
printf '%s\n' 'mod binary_object;' > "$fixture/src/runtime.rs"
cp -- "$repository_root/src/bytecode.rs" "$fixture/src/bytecode.rs"
cp -- "$repository_root/src/vm.rs" "$fixture/src/vm.rs"
cp -- "$repository_root/src/value.rs" "$fixture/src/value.rs"
cp -- "$repository_root/src/atom.rs" "$fixture/src/atom.rs"
cp -- "$repository_root/src/function.rs" "$fixture/src/function.rs"
cp -- "$repository_root/src/runtime/context.rs" "$fixture/src/runtime/context.rs"
cp -- "$repository_root/src/runtime/bytecode_publish.rs" \
    "$fixture/src/runtime/bytecode_publish.rs"
printf '%s\n' \
    'mod atoms;' \
    'mod code;' \
    'mod function_envelope;' \
    'mod function_translate;' \
    'mod bytecode_image;' \
    'mod graph;' \
    'mod pinned_atoms;' \
    'mod pinned_opcodes;' \
    'mod read_cursor;' \
    'mod ordinary_leaf;' \
    'mod scalar_script;' \
    'mod wire;' \
    'pub(super) use scalar_script::{ScalarScriptReadError, ScalarStringDraft, ScalarUnaryOp, ScalarValueDraft, decode_trusted_scalar_script};' \
    'pub(super) use ordinary_leaf::{DetachedAtomName, DetachedPrimitive, OrdinaryLeafApplyKind, OrdinaryLeafBinaryOp, OrdinaryLeafDraft, OrdinaryLeafMetadataDraft, OrdinaryLeafOp, OrdinaryLeafPredicateOp, OrdinaryLeafReadError, OrdinaryLeafStackOp, OrdinaryLeafUnaryOp, RootFunctionConstantSelector, decode_trusted_ordinary_leaf};' \
    > "$fixture/src/runtime/binary_object/mod.rs"
cp -- "$repository_root/src/runtime/binary_object/function_translate/mod.rs" \
    "$fixture/src/runtime/binary_object/function_translate/mod.rs"
cp -- "$repository_root/src/runtime/binary_object/function_translate/capability.rs" \
    "$fixture/src/runtime/binary_object/function_translate/capability.rs"
cp -- "$repository_root/src/runtime/binary_object/function_translate/dto.rs" \
    "$fixture/src/runtime/binary_object/function_translate/dto.rs"
python3 - \
    "$repository_root/src/runtime/binary_object/scalar_script.rs" \
    "$fixture/src/runtime/binary_object/scalar_script.rs" <<'PY_FIXTURE'
from pathlib import Path
import sys


source_path = Path(sys.argv[1])
target_path = Path(sys.argv[2])
source = source_path.read_text(encoding="utf-8")
test_module = "\n#[cfg(test)]\nmod tests {"
if source.count(test_module) != 1:
    raise SystemExit(
        "error: scalar_script.rs must contain exactly one cfg(test) module boundary"
    )
target_path.write_text(source.split(test_module, 1)[0] + "\n", encoding="utf-8")
PY_FIXTURE
python3 - \
    "$repository_root/src/runtime/binary_object/ordinary_leaf.rs" \
    "$fixture/src/runtime/binary_object/ordinary_leaf.rs" <<'PY_ORDINARY_FIXTURE'
from pathlib import Path
import sys


source_path = Path(sys.argv[1])
target_path = Path(sys.argv[2])
source = source_path.read_text(encoding="utf-8")
test_module = "\n#[cfg(test)]\nmod tests {"
if source.count(test_module) != 1:
    raise SystemExit(
        "error: ordinary_leaf.rs must contain exactly one cfg(test) module boundary"
    )
target_path.write_text(source.split(test_module, 1)[0] + "\n", encoding="utf-8")
PY_ORDINARY_FIXTURE
printf '%s\n' '// no alternate binary-object consumers' \
    > "$fixture/src/runtime/other.rs"
printf '%s\n' \
    'fn retained_from_raw(raw: u32) {' \
    '    let _ = PinnedAtomId::from_raw(raw);' \
    '}' \
    > "$fixture/src/runtime/binary_object/atoms.rs"
printf '%s\n' \
    'mod sealed {' \
    '    pub trait Sealed {}' \
    "    impl Sealed for WireCursor<'_> {}" \
    "    impl Sealed for SabTransportCursor<'_> {}" \
    '}' \
    "pub(in crate::runtime::binary_object) trait CheckedReadCursor<'input>: sealed::Sealed {}" \
    "impl<'input> CheckedReadCursor<'input> for WireCursor<'input> {}" \
    "impl<'input> CheckedReadCursor<'input> for SabTransportCursor<'input> {}" \
    > "$fixture/src/runtime/binary_object/read_cursor.rs"
printf '%s\n' \
    'mod atoms;' \
    'mod budget;' \
    'mod decode;' \
    'mod encode;' \
    'mod model;' \
    'mod native_plan;' \
    '#[cfg(test)]' \
    'mod tests;' \
    'pub(in crate::runtime::binary_object) use native_plan::{NativeAtomClass, NativeAtomRef, NativeCodePlan, NativeOperands, decode_native_code_plan};' \
    > "$fixture/src/runtime/binary_object/bytecode_image/mod.rs"
cp -- "$repository_root/src/runtime/binary_object/bytecode_image/native_plan.rs" \
    "$fixture/src/runtime/binary_object/bytecode_image/native_plan.rs"
cp -- "$repository_root/src/runtime/binary_object/pinned_opcodes.rs" \
    "$fixture/src/runtime/binary_object/pinned_opcodes.rs"
printf '%s\n' \
    'pub(in crate::runtime::binary_object) fn decode_bytecode_image_body() {}' \
    > "$fixture/src/runtime/binary_object/bytecode_image/decode/mod.rs"
printf '%s\n' \
    'pub(super) enum ImageAtom {' \
    '    Null,' \
    '    Index(u32),' \
    '    Predefined(PinnedAtomId),' \
    '    Dynamic(AtomId),' \
    '}' \
    > "$fixture/src/runtime/binary_object/bytecode_image/atoms.rs"
printf '%s\n' \
    'const PINNED_EVAL_ATOM_RAW: u32 = 84;' \
    'struct ImageLocalVariable { name: ImageAtom }' \
    'impl ImageLocalVariable {' \
    '    pub(in crate::runtime::binary_object) const fn name_is_null(&self) -> bool {' \
    '        matches!(self.name, ImageAtom::Null)' \
    '    }' \
    '}' \
    'struct ImageFunctionEnvelope { name: ImageAtom }' \
    'impl ImageFunctionEnvelope {' \
    '    pub(in crate::runtime::binary_object) const fn name_is_pinned_eval(&self) -> bool {' \
    '        match self.name {' \
    '            ImageAtom::Predefined(atom) => atom.raw() == PINNED_EVAL_ATOM_RAW,' \
    '            ImageAtom::Null | ImageAtom::Index(_) | ImageAtom::Dynamic(_) => false,' \
    '        }' \
    '    }' \
    '}' \
    'impl BytecodeImage {' \
    '    fn sab_archive_occurrences(&self) {}' \
    '}' \
    > "$fixture/src/runtime/binary_object/bytecode_image/model.rs"
printf '%s\n' \
    'pub(super) fn decode_graph_body() {}' \
    > "$fixture/src/runtime/binary_object/graph/decode.rs"
printf '%s\n' \
    'pub(in crate::runtime) struct NativeSabToken {' \
    '    native_token_bits: u64,' \
    '}' \
    '#[cfg(test)]' \
    'impl NativeSabToken {' \
    '    #[must_use]' \
    '    pub(in crate::runtime::binary_object) const fn from_test_bits(bits: u64) -> Self {' \
    '        Self {' \
    '            native_token_bits: bits,' \
    '        }' \
    '    }' \
    '}' \
    'pub(in crate::runtime) struct SabTransportInput<'"'"'a> {' \
    '    transport_wire_bytes: &'"'"'a [u8],' \
    '    transport_writer_occurrences: &'"'"'a [NativeSabToken],' \
    '}' \
    "impl<'a> SabTransportInput<'a> {" \
    '    #[must_use]' \
    '    pub(in crate::runtime) const fn new(' \
    '        wire: &'"'"'a [u8],' \
    '        writer_occurrences: &'"'"'a [NativeSabToken],' \
    '    ) -> Self {' \
    '        Self {' \
    '            transport_wire_bytes: wire,' \
    '            transport_writer_occurrences: writer_occurrences,' \
    '        }' \
    '    }' \
    '    fn build_cursor(' \
    '        self,' \
    '        mode: ReaderMode,' \
    '        wire_limits: WireLimits,' \
    '        graph_limits: GraphLimits,' \
    '    ) -> Result<SabTransportCursor<'"'"'a>, SabArchiveError> {' \
    '        Ok(SabTransportCursor {' \
    '            cursor_wire: WireCursor::new(self.transport_wire_bytes, mode, wire_limits)?,' \
    '            cursor_writer_occurrences: self.transport_writer_occurrences,' \
    '            cursor_next_occurrence: 0,' \
    '            cursor_archive: SabArchiveState::new(graph_limits),' \
    '        })' \
    '    }' \
    '    #[cfg(test)]' \
    '    fn into_cursor_for_test(' \
    '        self,' \
    '        mode: ReaderMode,' \
    '        wire_limits: WireLimits,' \
    '        graph_limits: GraphLimits,' \
    '    ) -> Result<SabTransportCursor<'"'"'a>, SabArchiveError> {' \
    '        self.build_cursor(mode, wire_limits, graph_limits)' \
    '    }' \
    '}' \
    'pub(in crate::runtime) fn decode_graph_with_sab_transport(' \
    '    input: SabTransportInput<'"'"'_>,' \
    '    mode: ReaderMode,' \
    '    wire_limits: WireLimits,' \
    '    graph_limits: GraphLimits,' \
    '    allow_object_references: bool,' \
    ') -> Result<ArchivedWireGraph, DecodeError> {' \
    '    let cursor = input' \
    '        .build_cursor(mode, wire_limits, graph_limits)' \
    '        .map_err(map_sab_archive_error)?;' \
    '    let (cursor, graph) =' \
    '        decode_graph_body(cursor, graph_limits, allow_object_references)?;' \
    '    cursor' \
    '        .finish_graph_archive(graph)' \
    '        .map_err(map_sab_archive_error)' \
    '}' \
    'pub(in crate::runtime) fn decode_bytecode_image_with_sab_transport(' \
    '    input: SabTransportInput<'"'"'_>,' \
    '    mode: ReaderMode,' \
    '    wire_limits: WireLimits,' \
    '    limits: BytecodeImageLimits,' \
    '    allow_object_references: bool,' \
    ') -> Result<ArchivedBytecodeImage, BytecodeImageError> {' \
    '    let cursor = input.build_cursor(mode, wire_limits, limits.graph())?;' \
    '    let (cursor, image) =' \
    '        decode_bytecode_image_body(cursor, limits, allow_object_references)?;' \
    '    cursor.finish_bytecode_image(image).map_err(Into::into)' \
    '}' \
    'pub(in crate::runtime::binary_object) struct SabTransportCursor<'"'"'a> {' \
    '    cursor_wire: WireCursor<'"'"'a>,' \
    '    cursor_writer_occurrences: &'"'"'a [NativeSabToken],' \
    '    cursor_next_occurrence: usize,' \
    '    cursor_archive: SabArchiveState,' \
    '}' \
    "impl<'a> SabTransportCursor<'a> {" \
    '    pub(in crate::runtime::binary_object) const fn position(&self) -> usize {' \
    '        let _ = &self.cursor_wire;' \
    '        0' \
    '    }' \
    '    pub(in crate::runtime::binary_object) const fn mode(&self) -> ReaderMode {' \
    '        let _ = &self.cursor_wire;' \
    '        mode' \
    '    }' \
    '    pub(in crate::runtime::binary_object) fn remaining(&self) -> usize {' \
    '        let _ = &self.cursor_wire;' \
    '        0' \
    '    }' \
    '    pub(in crate::runtime::binary_object) fn read_u8(&mut self) {' \
    '        let _ = &self.cursor_wire;' \
    '    }' \
    '    pub(in crate::runtime::binary_object) fn read_u16_le(&mut self) {' \
    '        let _ = &self.cursor_wire;' \
    '    }' \
    '    pub(in crate::runtime::binary_object) fn read_bytes(&mut self) {' \
    '        let _ = &self.cursor_wire;' \
    '    }' \
    '    pub(in crate::runtime::binary_object) fn read_tag(&mut self) {' \
    '        let _ = &self.cursor_wire;' \
    '    }' \
    '    pub(in crate::runtime::binary_object) fn read_uleb128(&mut self) {' \
    '        let _ = &self.cursor_wire;' \
    '    }' \
    '    pub(in crate::runtime::binary_object) fn read_i32(&mut self) {' \
    '        let _ = &self.cursor_wire;' \
    '    }' \
    '    pub(in crate::runtime::binary_object) fn read_f64(&mut self) {' \
    '        let _ = &self.cursor_wire;' \
    '    }' \
    '    pub(in crate::runtime::binary_object) fn read_header(&mut self) {' \
    '        let _ = &self.cursor_wire;' \
    '    }' \
    '    pub(in crate::runtime::binary_object) fn read_string(&mut self) {' \
    '        let _ = &self.cursor_wire;' \
    '    }' \
    '    pub(in crate::runtime::binary_object) fn validate_wire_end(&self) {' \
    '        let _ = &self.cursor_wire;' \
    '    }' \
    '    pub(super) fn record_shared_array_buffer(&mut self, expected: &NativeSabToken) {' \
    '        let _ = &self.cursor_wire;' \
    '        let _ = &self.cursor_wire;' \
    '        let _ = &self.cursor_writer_occurrences;' \
    '        let _ = &self.cursor_writer_occurrences;' \
    '        let _ = self.cursor_next_occurrence;' \
    '        let _ = self.cursor_next_occurrence;' \
    '        let _ = &self.cursor_archive;' \
    '        let _ = expected.native_token_bits;' \
    '    }' \
    '    fn finish_shared_backings(&self) -> Box<[SharedBackingDescriptor]> {' \
    '        let _ = &self.cursor_wire;' \
    '        let _ = &self.cursor_writer_occurrences;' \
    '        let _ = &self.cursor_writer_occurrences;' \
    '        let _ = self.cursor_next_occurrence;' \
    '        let _ = self.cursor_next_occurrence;' \
    '        let _ = &self.cursor_archive;' \
    '        shared_backings' \
    '    }' \
    '    fn finish_graph_archive(self, graph: WireGraph) -> Result<ArchivedWireGraph, Error> {' \
    '        let shared_backings = self.finish_shared_backings();' \
    '        ArchivedWireGraph {' \
    '            archived_graph_payload: graph,' \
    '            archived_graph_shared_backings: shared_backings,' \
    '        }' \
    '    }' \
    '    #[cfg(test)]' \
    '    fn finish_graph_archive_for_test(' \
    '        self,' \
    '        graph: WireGraph,' \
    '    ) -> Result<ArchivedWireGraph, SabArchiveError> {' \
    '        self.finish_graph_archive(graph)' \
    '    }' \
    '    fn finish_bytecode_image(self, image: BytecodeImage) -> Result<ArchivedBytecodeImage, Error> {' \
    '        let shared_backings = self.finish_shared_backings();' \
    '        image.sab_archive_occurrences();' \
    '        ArchivedBytecodeImage {' \
    '            archived_image_payload: image,' \
    '            archived_image_shared_backings: shared_backings,' \
    '        }' \
    '    }' \
    '}' \
    'pub(in crate::runtime) struct ArchivedWireGraph {' \
    '    archived_graph_payload: WireGraph,' \
    '    archived_graph_shared_backings: Box<[SharedBackingDescriptor]>,' \
    '}' \
    'impl ArchivedWireGraph {' \
    '    #[must_use]' \
    '    pub(in crate::runtime::binary_object) const fn shared_backing_count(&self) -> usize {' \
    '        self.archived_graph_shared_backings.len()' \
    '    }' \
    '    #[cfg(test)]' \
    '    pub(in crate::runtime::binary_object) const fn test_graph(&self) -> &WireGraph {' \
    '        &self.archived_graph_payload' \
    '    }' \
    '    #[cfg(test)]' \
    '    pub(super) fn test_shared_backing_descriptor(' \
    '        &self,' \
    '        backing: ArchiveBackingId,' \
    '    ) -> Option<SharedBackingDescriptor> {' \
    '        self.archived_graph_shared_backings' \
    '            .get(backing.as_usize())' \
    '            .copied()' \
    '    }' \
    '}' \
    'pub(in crate::runtime) struct ArchivedBytecodeImage {' \
    '    archived_image_payload: BytecodeImage,' \
    '    archived_image_shared_backings: Box<[SharedBackingDescriptor]>,' \
    '}' \
    'impl ArchivedBytecodeImage {' \
    '    #[must_use]' \
    '    pub(in crate::runtime::binary_object) const fn shared_backing_count(&self) -> usize {' \
    '        self.archived_image_shared_backings.len()' \
    '    }' \
    '    #[cfg(test)]' \
    '    pub(in crate::runtime::binary_object) const fn test_image(&self) -> &BytecodeImage {' \
    '        &self.archived_image_payload' \
    '    }' \
    '    #[cfg(test)]' \
    '    pub(in crate::runtime::binary_object) fn test_shared_backing_descriptor(' \
    '        &self,' \
    '        backing: ArchiveBackingId,' \
    '    ) -> Option<SharedBackingDescriptor> {' \
    '        self.archived_image_shared_backings' \
    '            .get(backing.as_usize())' \
    '            .copied()' \
    '    }' \
    '}' \
    > "$fixture/src/runtime/binary_object/graph/sab_transport.rs"

scan_root "$fixture" \
    || die "binary-object boundary rejected its clean no-consumer self-test fixture"

printf '%s\n' 'mod binary_object_publish;' >> "$fixture/src/runtime.rs"
cp -- "$repository_root/src/runtime/binary_object_publish.rs" \
    "$fixture/src/runtime/binary_object_publish.rs"

scan_root "$fixture" \
    || die "binary-object boundary rejected its clean sole-consumer self-test fixture"

expect_rejected() {
    local label=$1
    local diagnostic=$2
    local relative=$3
    local canary=$4
    local case_root=$tmp_dir/$label
    local output=$case_root.output

    mkdir -p "$case_root"
    cp -R "$fixture/." "$case_root"
    printf '\n%s\n' "$canary" >> "$case_root/$relative"
    if "$script_dir/check-binary-object-boundary.sh" --scan-only "$case_root" \
        > "$output" 2>&1; then
        die "binary-object boundary canary escaped: $label"
    fi
    if [[ $(<"$output") != *"$diagnostic"* ]]; then
        echo "error: binary-object boundary canary failed for the wrong reason: $label" >&2
        cat "$output" >&2
        exit 1
    fi
}

expect_rewrite_rejected() {
    local label=$1
    local diagnostic=$2
    local relative=$3
    local before=$4
    local after=$5
    local case_root=$tmp_dir/$label
    local output=$case_root.output

    mkdir -p "$case_root"
    cp -R "$fixture/." "$case_root"
    python3 - "$case_root/$relative" "$before" "$after" <<'PY'
from pathlib import Path
import sys

path = Path(sys.argv[1])
before = sys.argv[2]
after = sys.argv[3]
source = path.read_text(encoding="utf-8")
if source.count(before) != 1:
    raise SystemExit(f"rewrite canary expected one occurrence of {before!r}")
path.write_text(source.replace(before, after), encoding="utf-8")
PY
    if "$script_dir/check-binary-object-boundary.sh" --scan-only "$case_root" \
        > "$output" 2>&1; then
        die "binary-object boundary rewrite canary escaped: $label"
    fi
    if [[ $(<"$output") != *"$diagnostic"* ]]; then
        echo "error: binary-object boundary rewrite canary failed for the wrong reason: $label" >&2
        cat "$output" >&2
        exit 1
    fi
}

expect_full_rewrite_rejected() {
    local label=$1
    local diagnostic=$2
    local relative=$3
    local before=$4
    local after=$5
    local before2=${6-}
    local after2=${7-}
    local added_relative=${8-}
    local added_source=${9-}
    local case_root=$tmp_dir/$label
    local output=$case_root.output

    mkdir -p "$case_root"
    cp -R "$repository_root/src" "$case_root/src"
    mkdir -p "$case_root/tests/fixtures" "$case_root/dev-support/test262" "$case_root/docs"
    cp -- "$repository_root/Cargo.toml" "$case_root/Cargo.toml"
    cp -- "$repository_root/tests/fixtures/function_bytecode_wire.c" \
        "$case_root/tests/fixtures/function_bytecode_wire.c"
    cp -- "$repository_root/tests/fixtures/function_bytecode_wire.quickjs-2026-06-04.txt" \
        "$case_root/tests/fixtures/function_bytecode_wire.quickjs-2026-06-04.txt"
    cp -- "$repository_root/dev-support/quickjs-c-oracles.tsv" \
        "$case_root/dev-support/quickjs-c-oracles.tsv"
    cp -- "$repository_root/dev-support/test262/current.conf" \
        "$case_root/dev-support/test262/current.conf"
    cp -- "$repository_root/docs/status.md" "$case_root/docs/status.md"
    cp -- "$repository_root/tests/test262-class-private-callables-b-global-candidate.tsv" \
        "$case_root/tests/test262-class-private-callables-b-global-candidate.tsv"
    cp -- "$repository_root/tests/test262-class-private-callables-b-global-candidate.jsonl" \
        "$case_root/tests/test262-class-private-callables-b-global-candidate.jsonl"
    python3 - "$case_root/$relative" "$before" "$after" "$before2" "$after2" \
        "$case_root" "$added_relative" "$added_source" <<'PY'
from pathlib import Path
import sys

path = Path(sys.argv[1])
before = sys.argv[2]
after = sys.argv[3]
source = path.read_text(encoding="utf-8")
if source.count(before) != 1:
    raise SystemExit(f"full rewrite canary expected one occurrence of {before!r}")
source = source.replace(before, after)
before2 = sys.argv[4]
after2 = sys.argv[5]
if before2:
    if source.count(before2) != 1:
        raise SystemExit(f"full rewrite canary expected one occurrence of {before2!r}")
    source = source.replace(before2, after2)
path.write_text(source, encoding="utf-8")
case_root = Path(sys.argv[6])
added_relative = sys.argv[7]
if added_relative:
    added_path = case_root / added_relative
    if added_path.exists():
        raise SystemExit(f"full rewrite canary added path already exists: {added_relative!r}")
    added_path.parent.mkdir(parents=True, exist_ok=True)
    added_path.write_text(sys.argv[8], encoding="utf-8")
PY
    if "$script_dir/check-binary-object-boundary.sh" --scan-only "$case_root" \
        > "$output" 2>&1; then
        die "binary-object boundary full rewrite canary escaped: $label"
    fi
    if [[ $(<"$output") != *"$diagnostic"* ]]; then
        echo "error: binary-object boundary full rewrite canary failed for the wrong reason: $label" >&2
        cat "$output" >&2
        exit 1
    fi
}

expect_full_rewrite_table() {
    local label diagnostic relative before after
    while IFS='|' read -r label diagnostic relative before after; do
        [[ -n $label ]] || continue
        expect_full_rewrite_rejected "$label" "$diagnostic" "$relative" \
            "$(printf '%b' "$before")" "$(printf '%b' "$after")"
    done
}

expect_rejected vm-dependency forbidden-vm-dependency \
    src/runtime/binary_object/atoms.rs \
    'use crate::vm::Completion;'
expect_rejected compiler-dependency forbidden-compiler-dependency \
    src/runtime/binary_object/atoms.rs \
    'use crate::{atom::Atom, compiler as parser};'
expect_rejected heap-dependency forbidden-heap-dependency \
    src/runtime/binary_object/atoms.rs \
    'use crate::heap as runtime_heap;'
expect_rejected grouped-heap-dependency forbidden-heap-dependency \
    src/runtime/binary_object/atoms.rs \
    'use crate::{atom::Atom, heap as runtime_heap};'
expect_rejected runtime-dependency forbidden-runtime-dependency \
    src/runtime/binary_object/atoms.rs \
    'use crate::runtime as engine_runtime;'
expect_rejected runtime-representation forbidden-runtime-representation \
    src/runtime/binary_object/atoms.rs \
    'type Published = FunctionBytecodeData;'
expect_rejected codec-publication forbidden-publication-boundary \
    src/runtime/binary_object/atoms.rs \
    'fn publish(runtime: Runtime) { runtime.publish_unlinked_function(realm, function); }'
expect_rejected shared-memory-dependency forbidden-shared-memory-dependency \
    src/runtime/binary_object/atoms.rs \
    'use crate::shared_memory as runtime_shared_memory;'
expect_rejected parent-shared-memory-dependency forbidden-shared-memory-dependency \
    src/runtime/binary_object/atoms.rs \
    'use super::shared_memory as runtime_shared_memory;'
expect_rejected grouped-shared-memory-dependency forbidden-shared-memory-dependency \
    src/runtime/binary_object/atoms.rs \
    'use crate::{atom::Atom, shared_memory as runtime_shared_memory};'
expect_rejected parent-grouped-shared-memory-dependency forbidden-shared-memory-dependency \
    src/runtime/binary_object/atoms.rs \
    'use super::{atoms, shared_memory as runtime_shared_memory};'
expect_rejected shared-buffer-handle forbidden-shared-memory-runtime-type \
    src/runtime/binary_object/atoms.rs \
    'type RuntimeBacking = SharedBufferHandle;'
expect_rejected shared-backing-store forbidden-shared-memory-runtime-type \
    src/runtime/binary_object/atoms.rs \
    'type RuntimeBacking = SharedBackingStore;'
expect_rejected unsafe-block forbidden-unsafe-code \
    src/runtime/binary_object/atoms.rs \
    'fn bridge() { unsafe { core::hint::unreachable_unchecked() } }'
expect_rejected unsafe-function forbidden-unsafe-code \
    src/runtime/binary_object/atoms.rs \
    'unsafe fn bridge() {}'
expect_rejected unsafe-impl forbidden-unsafe-code \
    src/runtime/binary_object/atoms.rs \
    'unsafe impl Send for Archive {}'
expect_rejected unsafe-trait forbidden-unsafe-code \
    src/runtime/binary_object/atoms.rs \
    'unsafe trait NativeArchive {}'
expect_rejected non-null-pointer forbidden-non-null-pointer \
    src/runtime/binary_object/atoms.rs \
    'type NativeAddress = core::ptr::NonNull<u8>;'
expect_rejected raw-const-pointer forbidden-raw-pointer-type \
    src/runtime/binary_object/atoms.rs \
    'type NativeAddress = *const u8;'
expect_rejected raw-mut-pointer forbidden-raw-pointer-type \
    src/runtime/binary_object/atoms.rs \
    'type NativeAddress = *mut u8;'
expect_rejected from-raw-parts forbidden-native-pointer-bridge \
    src/runtime/binary_object/atoms.rs \
    'let bytes = core::slice::from_raw_parts(address, length);'
expect_rejected from-raw-parts-mut forbidden-native-pointer-bridge \
    src/runtime/binary_object/atoms.rs \
    'let bytes = core::slice::from_raw_parts_mut(address, length);'
expect_rejected into-raw forbidden-native-pointer-bridge \
    src/runtime/binary_object/atoms.rs \
    'let address = Box::into_raw(value);'
expect_rejected qualified-from-raw forbidden-native-pointer-bridge \
    src/runtime/binary_object/atoms.rs \
    'let value = Box::from_raw(address);'
expect_rejected bytecode-function forbidden-bytecode-function \
    src/runtime/binary_object/atoms.rs \
    'use crate::bytecode::BytecodeFunction;'
expect_rejected function-bytecode-ref forbidden-bytecode-function \
    src/runtime/binary_object/atoms.rs \
    'use crate::bytecode::FunctionBytecodeRef;'
expect_rejected public-lib public-lib-boundary \
    src/lib.rs \
    'pub use runtime::binary_object;'
expect_rejected lib-path-alias public-lib-boundary \
    src/lib.rs \
    '#[path = "runtime/binary_object/mod.rs"] pub mod archive;'
expect_rejected runtime-consumer runtime-boundary \
    src/runtime.rs \
    'use self::binary_object::bytecode_image::decode_bytecode_image;'
expect_rewrite_rejected consumer-public-module binary-object-consumer-module \
    src/runtime.rs \
    'mod binary_object_publish;' \
    'pub(super) mod binary_object_publish;'
expect_rejected second-binary-object-consumer binary-object-consumer-set \
    src/runtime/other.rs \
    'use super::binary_object::{ScalarValueDraft, decode_trusted_scalar_script};'
expect_rejected alternate-binary-object-path binary-object-consumer-set \
    src/runtime/other.rs \
    '#[path = "binary_object/mod.rs"] mod alternate_archive;'
expect_rejected second-scalar-facade-consumer binary-object-facade-consumer-set \
    src/runtime/other.rs \
    'fn leak() { let _ = decode_trusted_scalar_script(bytes); }'
expect_rejected consumer-codec-import binary-object-consumer-import \
    src/runtime/binary_object_publish.rs \
    'use super::binary_object::BytecodeImage;'
expect_rejected consumer-atom-string binary-object-consumer-atom-string \
    src/runtime/binary_object_publish.rs \
    'fn atom_constant(value: JsString) { let _ = UnlinkedConstant::atom_string(value); }'
expect_rejected consumer-atom-string-alias binary-object-consumer-atom-string \
    src/runtime/binary_object_publish.rs \
    'type C = UnlinkedConstant; fn aliased_atom_constant(value: JsString) { let _ = C::atom_string(value); }'
expect_rejected consumer-atom-interning binary-object-consumer-atom-interning \
    src/runtime/binary_object_publish.rs \
    'fn intern_directly(runtime: &Runtime) { let _ = runtime.intern_property_key("forbidden"); }'
expect_rejected consumer-second-publisher binary-object-consumer-publication \
    src/runtime/binary_object_publish.rs \
    'fn publish_twice(runtime: &Runtime) { let _ = runtime.publish_unlinked_function(realm, function); }'
expect_rejected consumer-verifier-bypass binary-object-consumer-publication \
    src/runtime/binary_object_publish.rs \
    'fn bypass(runtime: &Runtime) { runtime.publish_verified_unlinked_function(realm, function); }'
expect_rejected consumer-dead-safe-alternate-publication binary-object-consumer-alternate-entrypoint \
    src/runtime/binary_object_publish.rs \
    'fn alternate(runtime: &Runtime) { if false { let _ = runtime.publish_unlinked_function(realm, function); } let _ = runtime.compile_in_realm(realm, source); }'
expect_rejected consumer-heap-type binary-object-consumer-heap-type \
    src/runtime/binary_object_publish.rs \
    'fn allocate_directly(value: FunctionBytecodeData) {}'
expect_rejected consumer-root-forge binary-object-consumer-root-forge \
    src/runtime/binary_object_publish.rs \
    'fn forge(runtime: Runtime, id: FunctionBytecodeId) { let _ = FunctionBytecodeRef::from_owned_handle(runtime, id); }'
expect_rewrite_rejected consumer-lowered-scalar-unary-vector binary-object-consumer-scalar-mapping \
    src/runtime/binary_object_publish.rs \
    '    IntegerAtomString(u32),' \
    $'    IntegerAtomString(u32),\n    Unary(Vec<Instruction>),'
expect_rewrite_rejected consumer-float-normalization binary-object-consumer-float64 \
    src/runtime/binary_object_publish.rs \
    'lower_primitive_constant(Value::Float(f64::from_bits(bits)))' \
    'lower_primitive_constant(Value::number(f64::from_bits(bits)))'
expect_rewrite_rejected consumer-pool-atom-swap binary-object-consumer-scalar-mapping \
    src/runtime/binary_object_publish.rs \
    $'        ScalarValueDraft::ConstantString(value) => lower_scalar_string(value)\n            .and_then(|value| lower_primitive_constant(Value::String(value)))\n            .map(LoweredScalar::Constant),' \
    $'        ScalarValueDraft::ConstantString(value) => Ok(LoweredScalar::AtomString(\n            UnlinkedConstant::atom_string(lower_scalar_string(value)?),\n        )),'
expect_rewrite_rejected consumer-integer-via-cpool binary-object-consumer-scalar-mapping \
    src/runtime/binary_object_publish.rs \
    'ScalarValueDraft::IntegerAtomString(value) => Ok(LoweredScalar::IntegerAtomString(value)),' \
    'ScalarValueDraft::IntegerAtomString(value) => lower_primitive_constant(Value::String(JsString::from_fresh_decimal_u32(value))).map(LoweredScalar::Constant),'
expect_rewrite_rejected consumer-empty-primitive binary-object-consumer-scalar-mapping \
    src/runtime/binary_object_publish.rs \
    $'        ScalarValueDraft::EmptyString => Ok(LoweredScalar::AtomString(\n            UnlinkedConstant::atom_string(JsString::from_static("")),\n        )),' \
    $'        ScalarValueDraft::EmptyString => Ok(LoweredScalar::Constant(\n            lower_primitive_constant(Value::String(JsString::from_static("")))?,\n        )),'
expect_rewrite_rejected consumer-bigint-dead-path-coercion binary-object-consumer-scalar-mapping \
    src/runtime/binary_object_publish.rs \
    $'        ScalarValueDraft::BigIntBytes(bytes) => {\n            lower_bigint_constant(&bytes).map(LoweredScalar::Constant)\n        }' \
    $'        ScalarValueDraft::BigIntBytes(bytes) => {\n            if false { return lower_bigint_constant(&bytes).map(LoweredScalar::Constant); }\n            Ok(LoweredScalar::Direct(Instruction::PushI32(i32::from(bytes.first().copied().unwrap_or(0)))))\n        }'
expect_rewrite_rejected consumer-bigint-noncanonical-decode binary-object-consumer-bigint \
    src/runtime/binary_object_publish.rs \
    'JsBigInt::decode_bc5_signed_le(bytes, bytes.len(), bytes.len(), true)' \
    'JsBigInt::decode_bc5_signed_le(bytes, bytes.len(), bytes.len(), false)'
expect_rewrite_rejected consumer-bigint-partial-consumption binary-object-consumer-bigint \
    src/runtime/binary_object_publish.rs \
    '    if consumed != bytes.len() {' \
    '    if false {'
expect_rewrite_rejected consumer-bigint-input-shadow binary-object-consumer-bigint \
    src/runtime/binary_object_publish.rs \
    '    let (value, consumed) =' \
    $'    let bytes = &bytes[..1];\n    let (value, consumed) ='
expect_rewrite_rejected consumer-unary-negation-mapping-drift binary-object-consumer-publication \
    src/runtime/binary_object_publish.rs \
    '                ScalarUnaryOp::Neg => Instruction::Neg,' \
    '                ScalarUnaryOp::Neg => Instruction::Nop,'
expect_rewrite_rejected consumer-unary-chain-reorder binary-object-consumer-publication \
    src/runtime/binary_object_publish.rs \
    '        for operation in unary_ops {' \
    '        for operation in unary_ops.into_iter().rev() {'
expect_rewrite_rejected consumer-unary-eager-precompute binary-object-consumer-publication \
    src/runtime/binary_object_publish.rs \
    '        let (value, unary_ops) = decode_trusted_scalar_script(bytes).map_err(map_read_error)?;' \
    $'        let (value, unary_ops) = decode_trusted_scalar_script(bytes).map_err(map_read_error)?;\n        let value = match value { ScalarValueDraft::Int(value) => ScalarValueDraft::Int(value.wrapping_neg()), value => value };'
expect_rejected consumer-bigint-eager-negation binary-object-consumer-bigint-eager-negation \
    src/runtime/binary_object_publish.rs \
    'fn eager_negation(value: JsBigInt) { let _ = std::ops::Neg::neg(value); }'
expect_rewrite_rejected consumer-skips-safe-publication binary-object-consumer-publication \
    src/runtime/binary_object_publish.rs \
    '        self.publish_unlinked_function(realm, function)' \
    '        self.compile_in_realm(realm, source)'
expect_rewrite_rejected ordinary-consumer-float-normalization ordinary-leaf-consumer-lowering \
    src/runtime/binary_object_publish.rs \
    'DetachedPrimitive::Float64Bits(bits) => Value::Float(f64::from_bits(bits)),' \
    'DetachedPrimitive::Float64Bits(bits) => Value::number(f64::from_bits(bits)),'
expect_rewrite_rejected ordinary-consumer-op-remap ordinary-leaf-consumer-lowering \
    src/runtime/binary_object_publish.rs \
    'OrdinaryLeafBinaryOp::Add => Instruction::Add,' \
    'OrdinaryLeafBinaryOp::Add => Instruction::Sub,'
expect_rewrite_rejected ordinary-consumer-verifier-dead-branch ordinary-leaf-consumer-publication \
    src/runtime/binary_object_publish.rs \
    $'        super::bytecode_publish::verify_unlinked_ordinary_leaf(&function)\n            .map_err(map_ordinary_leaf_verification_error)?;' \
    $'        if false {\n            super::bytecode_publish::verify_unlinked_ordinary_leaf(&function)\n                .map_err(map_ordinary_leaf_verification_error)?;\n        }'
expect_rewrite_rejected ordinary-consumer-generic-publisher ordinary-leaf-consumer-publication \
    src/runtime/binary_object_publish.rs \
    'self.publish_verified_unlinked_function(realm, function)?' \
    'self.publish_unlinked_function(realm, function)?'
expect_full_rewrite_rejected ordinary-consumer-raw-native-plan ordinary-leaf-consumer-import \
    src/runtime/binary_object_publish.rs \
    $'#[cfg(test)]\nmod tests {' \
    $'fn leak_native_plan(_: NativeCodePlan<'"'"'_>) {}\n\n#[cfg(test)]\nmod tests {'
expect_full_rewrite_rejected ordinary-consumer-test262-branch ordinary-leaf-consumer-special-casing \
    src/runtime/binary_object_publish.rs \
    $'#[cfg(test)]\nmod tests {' \
    $'fn fixture_dispatch(bytes: &[u8]) -> bool { bytes.starts_with(&[0x05, 0x00]) } // Test262 fixture\n\n#[cfg(test)]\nmod tests {'
expect_rejected root-public-module root-module-visibility \
    src/runtime/binary_object/mod.rs \
    'pub(in crate::runtime) mod leaked;'
expect_rewrite_rejected ordinary-leaf-public-module root-module-visibility \
    src/runtime/binary_object/mod.rs \
    'mod ordinary_leaf;' \
    'pub(super) mod ordinary_leaf;'
expect_rewrite_rejected scalar-script-public-module root-module-visibility \
    src/runtime/binary_object/mod.rs \
    'mod scalar_script;' \
    'pub(super) mod scalar_script;'
expect_rejected root-extra-private-module root-private-module-set \
    src/runtime/binary_object/mod.rs \
    'mod unreviewed_admission;'
expect_rejected root-reexport root-reexport \
    src/runtime/binary_object/mod.rs \
    'pub(in crate::runtime) use bytecode_image::*;'
expect_rewrite_rejected scalar-facade-extra-type scalar-script-facade-shape \
    src/runtime/binary_object/mod.rs \
    'pub(super) use scalar_script::{ScalarScriptReadError, ScalarStringDraft, ScalarUnaryOp, ScalarValueDraft, decode_trusted_scalar_script};' \
    'pub(super) use scalar_script::{BytecodeImage, ScalarScriptReadError, ScalarStringDraft, ScalarUnaryOp, ScalarValueDraft, decode_trusted_scalar_script};'
expect_rewrite_rejected scalar-facade-wider-visibility scalar-script-facade-shape \
    src/runtime/binary_object/mod.rs \
    'pub(super) use scalar_script::{ScalarScriptReadError, ScalarStringDraft, ScalarUnaryOp, ScalarValueDraft, decode_trusted_scalar_script};' \
    'pub(crate) use scalar_script::{ScalarScriptReadError, ScalarStringDraft, ScalarUnaryOp, ScalarValueDraft, decode_trusted_scalar_script};'
expect_rewrite_rejected ordinary-facade-extra-type ordinary-leaf-facade-shape \
    src/runtime/binary_object/mod.rs \
    'DetachedPrimitive, OrdinaryLeafApplyKind, OrdinaryLeafBinaryOp, OrdinaryLeafDraft,' \
    'BytecodeImage, DetachedPrimitive, OrdinaryLeafApplyKind, OrdinaryLeafBinaryOp, OrdinaryLeafDraft,'
expect_rewrite_rejected ordinary-facade-wider-visibility ordinary-leaf-facade-shape \
    src/runtime/binary_object/mod.rs \
    'pub(super) use ordinary_leaf::{' \
    'pub(crate) use ordinary_leaf::{'
expect_rejected ordinary-extra-visible-item ordinary-leaf-visible-item-set \
    src/runtime/binary_object/ordinary_leaf.rs \
    'pub(in crate::runtime) fn leak_archive_identity() {}'
expect_rejected ordinary-private-helper ordinary-leaf-helper-set \
    src/runtime/binary_object/ordinary_leaf.rs \
    'fn bypass_ordinary_admission() {}'
expect_rejected ordinary-raw-code-dependency ordinary-leaf-native-plan-boundary \
    src/runtime/binary_object/ordinary_leaf.rs \
    'fn leak_raw_code(_: &ImageCode) {}'
expect_rewrite_rejected ordinary-input-prefix-dispatch ordinary-leaf-special-casing \
    src/runtime/binary_object/ordinary_leaf.rs \
    '    if input.len() > MAX_INPUT_BYTES {' \
    '    if input.starts_with(&[0x05, 0x00]) || input.len() > MAX_INPUT_BYTES {'
expect_rewrite_rejected ordinary-verifier-strip-bypass ordinary-leaf-verifier-role \
    src/runtime/bytecode_publish.rs \
    '                        || !metadata.strip_variable_debug' \
    '                        || false'
expect_rewrite_rejected ordinary-verifier-debug-bypass ordinary-leaf-verifier-role \
    src/runtime/bytecode_publish.rs \
    '                        || function.debug().is_some()' \
    '                        || false'
expect_rewrite_rejected ordinary-verifier-primitive-broadening ordinary-leaf-plain-primitive \
    src/function.rs \
    'matches!(self.0, UnlinkedConstantKind::Primitive(_))' \
    'matches!(self.0, UnlinkedConstantKind::Primitive(_) | UnlinkedConstantKind::AtomString(_))'
expect_rewrite_rejected ordinary-verifier-empty-atom-broadening ordinary-leaf-plain-primitive \
    src/function.rs \
    'UnlinkedConstantKind::AtomString(Value::String(value)) if value.is_empty()' \
    'UnlinkedConstantKind::AtomString(Value::String(_))'
expect_rewrite_rejected ordinary-public-api-selector-collapse ordinary-leaf-public-api \
    src/runtime/context.rs \
    $'            bytes,\n            root_constant_index,\n        );' \
    $'            bytes,\n            0,\n        );'
expect_rewrite_rejected ordinary-public-api-pending-broadening ordinary-leaf-public-api \
    src/runtime/context.rs \
    $'            Ok(function) => Ok(function),\n            Err(RuntimeError::Engine(error))\n                if NativeErrorKind::from_javascript_error(error.kind()).is_some() =>' \
    $'            Ok(function) => Ok(function),\n            Err(RuntimeError::Engine(error))\n                if true || NativeErrorKind::from_javascript_error(error.kind()).is_some() =>'
expect_rewrite_rejected scalar-draft-copy-regression scalar-script-draft-shape \
    src/runtime/binary_object/scalar_script.rs \
    $'#[derive(Clone, Debug, Eq, PartialEq)]\npub(in crate::runtime) enum ScalarValueDraft' \
    $'#[derive(Clone, Copy, Debug, Eq, PartialEq)]\npub(in crate::runtime) enum ScalarValueDraft'
expect_rewrite_rejected scalar-unary-chain-storage scalar-script-sequence-shape \
    src/runtime/binary_object/scalar_script.rs \
    '    unary_ops: Box<[ScalarUnaryOp]>,' \
    '    unary_ops: Vec<ScalarUnaryOp>,'
expect_rejected scalar-opcode-set-widening scalar-script-opcode-set \
    src/runtime/binary_object/scalar_script.rs \
    'const OP_PUSH_THIS: u8 = 0x08;'
expect_rewrite_rejected scalar-unary-name-widening scalar-unary-operation-shape \
    src/runtime/binary_object/scalar_script.rs \
    '            FunctionUnaryOp::TypeOf => Self::TypeOf,' \
    $'            FunctionUnaryOp::TypeOf => Self::TypeOf,\n            FunctionUnaryOp::Neg => Self::TypeOf,'
expect_rejected translate-extra-module function-translate-module-set \
    src/runtime/binary_object/function_translate/escape.rs 'fn escape() {}'
expect_full_rewrite_table <<'TRANSLATE_CANARIES'
translate-registry-raw-drift|function-translate-registry-raw|src/runtime/binary_object/function_translate/capability.rs|    row!(155, None, OrdinaryOnly, Recipe::Binary(FunctionBinaryOp::Add)),|    row!(154, None, OrdinaryOnly, Recipe::Binary(FunctionBinaryOp::Add)),
translate-registry-format-drift|function-translate-registry-descriptor|src/runtime/binary_object/function_translate/capability.rs|    row!(155, None, OrdinaryOnly, Recipe::Binary(FunctionBinaryOp::Add)),|    row!(155, I32, OrdinaryOnly, Recipe::Binary(FunctionBinaryOp::Add)),
translate-registry-policy-swap|function-translate-registry-policy|src/runtime/binary_object/function_translate/capability.rs|    row!(150, None, Blocked, Operator),\n    row!(151, Atom, Blocked, Binding),\n    row!(152, None, OrdinaryOnly, Recipe::Binary(FunctionBinaryOp::Mul)),|    row!(150, None, OrdinaryOnly, Recipe::Binary(FunctionBinaryOp::Mul)),\n    row!(151, Atom, Blocked, Binding),\n    row!(152, None, Blocked, Operator),
translate-registry-recipe-remap|function-translate-registry-policy|src/runtime/binary_object/function_translate/capability.rs|    row!(155, None, OrdinaryOnly, Recipe::Binary(FunctionBinaryOp::Add)),|    row!(155, None, OrdinaryOnly, Recipe::Binary(FunctionBinaryOp::Sub)),
translate-registry-blocker-drift|function-translate-registry-blockers|src/runtime/binary_object/function_translate/capability.rs|    row!(5, Atom, Blocked, ValueConstruction),|    row!(5, Atom, Blocked, Property),
translate-call-registry-raw|function-translate-registry-raw|src/runtime/binary_object/function_translate/capability.rs|    row!(34, NPop, OrdinaryOnly, Recipe::Call),|    row!(35, NPop, OrdinaryOnly, Recipe::Call),
translate-call-registry-npop-format|function-translate-registry-descriptor|src/runtime/binary_object/function_translate/capability.rs|    row!(34, NPop, OrdinaryOnly, Recipe::Call),|    row!(34, NPopX, OrdinaryOnly, Recipe::Call),
translate-call0-registry-npopx-format|function-translate-registry-descriptor|src/runtime/binary_object/function_translate/capability.rs|    row!(236, NPopX, OrdinaryOnly, Recipe::Call),|    row!(236, NPop, OrdinaryOnly, Recipe::Call),
translate-call-registry-audience|function-translate-registry-audience|src/runtime/binary_object/function_translate/capability.rs|    row!(34, NPop, OrdinaryOnly, Recipe::Call),|    row!(34, NPop, Shared, Recipe::Call),
stage3a-construct-registry-audience|function-translate-registry-audience|src/runtime/binary_object/function_translate/capability.rs|    row!(33, NPop, OrdinaryOnly, Recipe::Construct),|    row!(33, NPop, Shared, Recipe::Construct),
stage3c-tail-call-registry-audience|function-translate-registry-audience|src/runtime/binary_object/function_translate/capability.rs|    row!(35, NPop, OrdinaryOnly, Recipe::TailCall),|    row!(35, NPop, Blocked, Completion),
stage3c-tail-call-method-registry-audience|function-translate-registry-audience|src/runtime/binary_object/function_translate/capability.rs|    row!(37, NPop, OrdinaryOnly, Recipe::TailCallMethod),|    row!(37, NPop, Blocked, Completion),
stage3b-apply-registry-audience|function-translate-registry-audience|src/runtime/binary_object/function_translate/capability.rs|    row!(39, U16, OrdinaryOnly, Recipe::Apply),|    row!(39, U16, Blocked, Completion),
stage3b-apply-recipe-payload|function-translate-recipe-shape|src/runtime/binary_object/function_translate/capability.rs|    Apply,\n    Return,|    Apply(u16),\n    Return,
translate-recipe-call-payload|function-translate-recipe-shape|src/runtime/binary_object/function_translate/capability.rs|    Call,|    Call(u16),
stage3c-tail-call-recipe-payload|function-translate-recipe-shape|src/runtime/binary_object/function_translate/capability.rs|    Call,\n    TailCall,\n    Construct,|    Call,\n    TailCall(u16),\n    Construct,
translate-dto-call-payload|function-translate-dto-shape|src/runtime/binary_object/function_translate/dto.rs|    Call(u16),|    Call(u32),
stage3c-function-tail-call-payload|function-translate-dto-shape|src/runtime/binary_object/function_translate/dto.rs|    Call(u16),\n    TailCall(u16),\n    Construct(u16),|    Call(u16),\n    TailCall(u32),\n    Construct(u16),
stage3b-function-apply-raw-payload|function-translate-dto-shape|src/runtime/binary_object/function_translate/dto.rs|    Apply(FunctionApplyKind),|    Apply(u16),
stage3b-function-apply-kind-widening|function-translate-apply-kind|src/runtime/binary_object/function_translate/dto.rs|pub(in crate::runtime::binary_object) enum FunctionApplyKind {\n    Call,\n    Construct,\n}|pub(in crate::runtime::binary_object) enum FunctionApplyKind {\n    Call,\n    Construct,\n    Raw(u16),\n}
ordinary-call-dto-payload|ordinary-leaf-operation-shape|src/runtime/binary_object/ordinary_leaf.rs|    Call(u16),|    Call(u32),
stage3c-ordinary-tail-method-payload|ordinary-leaf-operation-shape|src/runtime/binary_object/ordinary_leaf.rs|    CallMethod(u16),\n    TailCallMethod(u16),\n    ArrayFrom(u16),|    CallMethod(u16),\n    TailCallMethod(u32),\n    ArrayFrom(u16),
stage3b-ordinary-apply-raw-payload|ordinary-leaf-operation-shape|src/runtime/binary_object/ordinary_leaf.rs|    Apply(OrdinaryLeafApplyKind),|    Apply(u16),
stage3b-ordinary-apply-kind-widening|ordinary-leaf-apply-kind|src/runtime/binary_object/ordinary_leaf.rs|pub(in crate::runtime) enum OrdinaryLeafApplyKind {\n    Call,\n    Construct,\n}|pub(in crate::runtime) enum OrdinaryLeafApplyKind {\n    Call,\n    Construct,\n    Raw(u16),\n}
translate-blocker-bucket-revival|function-translate-dto-shape|src/runtime/binary_object/function_translate/dto.rs|    FunctionGraph,\n    Completion,|    FunctionGraph,\n    Invocation,\n    Completion,
translate-native-plan-second-consumer|native-plan-consumer-set|src/runtime/binary_object/scalar_script.rs|use std::fmt;|use super::bytecode_image::NativeCodePlan;\nuse std::fmt;
translate-dto-function-id-leak|function-translate-dto-representation|src/runtime/binary_object/function_translate/dto.rs|    value: AtomOperandValue<'image>,\n    from_input_atom_table: bool,|    value: AtomOperandValue<'image>,\n    function_id: FunctionId,\n    from_input_atom_table: bool,
translate-dto-wire-string-leak|function-translate-dto-representation|src/runtime/binary_object/function_translate/dto.rs|    value: AtomOperandValue<'image>,\n    from_input_atom_table: bool,|    value: AtomOperandValue<'image>,\n    wire: WireString,\n    from_input_atom_table: bool,
translate-dto-raw-opcode-leak|function-translate-dto-representation|src/runtime/binary_object/function_translate/dto.rs|    value: AtomOperandValue<'image>,\n    from_input_atom_table: bool,|    value: AtomOperandValue<'image>,\n    raw_opcode: u8,\n    from_input_atom_table: bool,
translate-dto-partial-eq|function-translate-dto-representation|src/runtime/binary_object/function_translate/dto.rs|#[derive(Clone, Copy)]\npub(in crate::runtime::binary_object) struct AtomOperand|#[derive(Clone, Copy, PartialEq)]\npub(in crate::runtime::binary_object) struct AtomOperand
translate-dto-cfg-partial-eq|function-translate-dto-representation|src/runtime/binary_object/function_translate/dto.rs|#[derive(Clone, Copy)]\npub(in crate::runtime::binary_object) struct AtomOperand|#[cfg_attr(not(test), derive(PartialEq))]\n#[derive(Clone, Copy)]\npub(in crate::runtime::binary_object) struct AtomOperand
translate-dto-hash|function-translate-dto-representation|src/runtime/binary_object/function_translate/dto.rs|#[derive(Clone, Copy)]\npub(in crate::runtime::binary_object) struct AtomOperand|#[derive(Clone, Copy, Hash)]\npub(in crate::runtime::binary_object) struct AtomOperand
translate-code-debug|function-translate-dto-representation|src/runtime/binary_object/function_translate/dto.rs|#[derive(Clone)]\npub(in crate::runtime::binary_object) struct FunctionCode|#[derive(Clone, Debug)]\npub(in crate::runtime::binary_object) struct FunctionCode
translate-code-default|function-translate-dto-representation|src/runtime/binary_object/function_translate/dto.rs|#[derive(Clone)]\npub(in crate::runtime::binary_object) struct FunctionCode|#[derive(Clone, Default)]\npub(in crate::runtime::binary_object) struct FunctionCode
translate-dto-constructor-remap|function-translate-dto-representation|src/runtime/binary_object/function_translate/dto.rs|        Self {\n            audience,\n            diagnostic,\n            operation,\n        }|        Self {\n            audience,\n            diagnostic,\n            operation: FunctionOp::OutsideTarget,\n        }
translate-diagnostic-semantic-dispatch|function-translate-semantic-dispatch|src/runtime/binary_object/function_translate/mod.rs|    let ready =|    let _ = OperationDiagnostic::new("forbidden", OperandShape::None);\n    let ready =
translate-diagnostic-extra-access|function-translate-diagnostic-boundary|src/runtime/binary_object/ordinary_leaf.rs|        if !instruction.supports_ordinary() {|        let _ = instruction.rejection_diagnostic();\n        if !instruction.supports_ordinary() {
translate-expansion-capacity|function-translate-expansion|src/runtime/binary_object/function_translate/mod.rs|    operations: [Option<PendingOperation<'image>>; 4],|    operations: [Option<PendingOperation<'image>>; 5],
translate-expansion-slot-swap|function-translate-expansion|src/runtime/binary_object/function_translate/mod.rs|            operations: [Some(first), Some(second), None, None],|            operations: [Some(second), Some(first), None, None],
translate-expansion-length-collapse|function-translate-expansion|src/runtime/binary_object/function_translate/mod.rs|        self.len as usize|        1
translate-expansion-iterator-reverse|function-translate-expansion|src/runtime/binary_object/function_translate/mod.rs|            .flatten()|            .flatten().rev()
translate-expansion-remap|function-translate-expansion|src/runtime/binary_object/function_translate/mod.rs|                PendingOperation::Ready(FunctionOp::Stack(FunctionStackOp::Perm5)),|                PendingOperation::Ready(FunctionOp::Stack(FunctionStackOp::Perm4)),
translate-return-undefined-expansion|function-translate-expansion|src/runtime/binary_object/function_translate/mod.rs|        (Recipe::ReturnUndefined, NativeOperands::None) => ready(FunctionOp::ReturnUndefined),|        (Recipe::ReturnUndefined, NativeOperands::None) => Ok(PendingExpansion::two(\n            PendingOperation::Ready(FunctionOp::PushUndefined),\n            PendingOperation::Ready(FunctionOp::Return),\n        )),
translate-undefined-null-swap|function-translate-semantic-dispatch|src/runtime/binary_object/function_translate/mod.rs|        (Recipe::PushUndefined, NativeOperands::None) => ready(FunctionOp::PushUndefined),\n        (Recipe::PushNull, NativeOperands::None) => ready(FunctionOp::PushNull),|        (Recipe::PushUndefined, NativeOperands::None) => ready(FunctionOp::PushNull),\n        (Recipe::PushNull, NativeOperands::None) => ready(FunctionOp::PushUndefined),
translate-false-payload|function-translate-semantic-dispatch|src/runtime/binary_object/function_translate/mod.rs|        (Recipe::PushFalse, NativeOperands::None) => ready(FunctionOp::PushBool(false)),|        (Recipe::PushFalse, NativeOperands::None) => ready(FunctionOp::PushBool(true)),
translate-second-pass-ready-remap|function-translate-control-flow|src/runtime/binary_object/function_translate/mod.rs|                PendingOperation::Ready(operation) => operation,|                PendingOperation::Ready(_) => FunctionOp::OutsideTarget,
translate-instruction-construction-remap|function-translate-control-flow|src/runtime/binary_object/function_translate/mod.rs|            output.push(FunctionInstruction::new(\n                instruction.audience,\n                instruction.diagnostic,\n                operation,\n            ));|            output.push(FunctionInstruction::new(\n                instruction.audience,\n                instruction.diagnostic,\n                FunctionOp::OutsideTarget,\n            ));
translate-branch-map-collapse|function-translate-control-flow|src/runtime/binary_object/function_translate/mod.rs|        source_to_output.push(output_index);|        source_to_output.push(0);
translate-branch-map-offset|function-translate-control-flow|src/runtime/binary_object/function_translate/mod.rs|        let output_index = u32::try_from(output_len)|        output_len = output_len.saturating_add(1);\n        let output_index = u32::try_from(output_len)
translate-branch-map-shadow|function-translate-control-flow|src/runtime/binary_object/function_translate/mod.rs|        source_to_output.push(output_index);|        let output_index = output_index.saturating_add(1);\n        source_to_output.push(output_index);
translate-branch-target-collapse|function-translate-control-flow|src/runtime/binary_object/function_translate/mod.rs|FunctionOp::IfFalse(resolve_target(&source_to_output, target)?)|FunctionOp::IfFalse(0)
translate-if-true-target-collapse|function-translate-control-flow|src/runtime/binary_object/function_translate/mod.rs|FunctionOp::IfTrue(resolve_target(&source_to_output, target)?)|FunctionOp::IfTrue(0)
translate-target-filter-bypass|function-translate-atom-order|src/runtime/binary_object/function_translate/mod.rs|    if target.accepts(audience) {|    if true {
translate-atom-allocation|function-translate-atom-order|src/runtime/binary_object/function_translate/mod.rs|    let from_input_atom_table = atom.originates_from_input_atom_table();|    let _scratch = Vec::<u8>::new();\n    let from_input_atom_table = atom.originates_from_input_atom_table();
translate-source-hash-dispatch|function-translate-special-casing|src/runtime/binary_object/function_translate/mod.rs|    let ready =|    let source_hash = 0_u64;\n    let ready =
ordinary-return-undefined-remap|ordinary-leaf-consumer-lowering|src/runtime/binary_object_publish.rs|        OrdinaryLeafOp::ReturnUndefined => Instruction::ReturnUndefined,|        OrdinaryLeafOp::ReturnUndefined => Instruction::Return,
ordinary-handoff-push-i32-payload|ordinary-leaf-translated-code|src/runtime/binary_object/ordinary_leaf.rs|        FunctionOp::PushI32(value) => Ok(OrdinaryLeafOp::PushI32(*value)),|        FunctionOp::PushI32(value) => Ok(OrdinaryLeafOp::PushI32(-*value)),
ordinary-handoff-get-local-remap|ordinary-leaf-translated-code|src/runtime/binary_object/ordinary_leaf.rs|        FunctionOp::GetLocal(index) => lower_local(*index, local_count, OrdinaryLeafOp::GetLocal),|        FunctionOp::GetLocal(index) => lower_local(*index, local_count, OrdinaryLeafOp::PutLocal),
ordinary-handoff-call-argc|ordinary-leaf-translated-code|src/runtime/binary_object/ordinary_leaf.rs|        FunctionOp::Call(argument_count) => Ok(OrdinaryLeafOp::Call(*argument_count)),|        FunctionOp::Call(argument_count) => Ok(OrdinaryLeafOp::Call(argument_count.saturating_add(1))),
stage3c-ordinary-tail-call-to-call|ordinary-leaf-translated-code|src/runtime/binary_object/ordinary_leaf.rs|        FunctionOp::TailCall(argument_count) => Ok(OrdinaryLeafOp::TailCall(*argument_count)),|        FunctionOp::TailCall(argument_count) => Ok(OrdinaryLeafOp::Call(*argument_count)),
stage3c-ordinary-tail-method-argc|ordinary-leaf-translated-code|src/runtime/binary_object/ordinary_leaf.rs|            Ok(OrdinaryLeafOp::TailCallMethod(*argument_count))|            Ok(OrdinaryLeafOp::TailCallMethod(argument_count.saturating_add(1)))
stage3a-ordinary-array-from-count-minus-one|ordinary-leaf-translated-code|src/runtime/binary_object/ordinary_leaf.rs|        FunctionOp::ArrayFrom(element_count) => Ok(OrdinaryLeafOp::ArrayFrom(*element_count)),|        FunctionOp::ArrayFrom(element_count) => Ok(OrdinaryLeafOp::ArrayFrom(element_count.saturating_sub(1))),
stage3a-ordinary-call-method-to-call|ordinary-leaf-translated-code|src/runtime/binary_object/ordinary_leaf.rs|        FunctionOp::CallMethod(argument_count) => Ok(OrdinaryLeafOp::CallMethod(*argument_count)),|        FunctionOp::CallMethod(argument_count) => Ok(OrdinaryLeafOp::Call(*argument_count)),
stage3b-ordinary-apply-kind-swap|ordinary-leaf-translated-code|src/runtime/binary_object/ordinary_leaf.rs|            FunctionApplyKind::Call => OrdinaryLeafApplyKind::Call,|            FunctionApplyKind::Call => OrdinaryLeafApplyKind::Construct,
ordinary-publisher-get-local-offset|ordinary-leaf-consumer-lowering|src/runtime/binary_object_publish.rs|        OrdinaryLeafOp::GetLocal(index) => Instruction::GetLocal(index),|        OrdinaryLeafOp::GetLocal(index) => Instruction::GetLocal(index.saturating_add(1)),
ordinary-publisher-call-argc|ordinary-leaf-consumer-lowering|src/runtime/binary_object_publish.rs|        OrdinaryLeafOp::Call(argument_count) => Instruction::Call(argument_count),|        OrdinaryLeafOp::Call(argument_count) => Instruction::Call(argument_count.saturating_add(1)),
ordinary-publisher-call-method|ordinary-leaf-consumer-lowering|src/runtime/binary_object_publish.rs|        OrdinaryLeafOp::Call(argument_count) => Instruction::Call(argument_count),|        OrdinaryLeafOp::Call(argument_count) => Instruction::CallMethod(argument_count),
ordinary-publisher-call-push-undefined|ordinary-leaf-consumer-lowering|src/runtime/binary_object_publish.rs|        OrdinaryLeafOp::Call(argument_count) => Instruction::Call(argument_count),|        OrdinaryLeafOp::Call(_argument_count) => Instruction::Undefined,
stage3c-publisher-tail-call-to-call|ordinary-leaf-consumer-lowering|src/runtime/binary_object_publish.rs|        OrdinaryLeafOp::TailCall(argument_count) => Instruction::TailCall(argument_count),|        OrdinaryLeafOp::TailCall(argument_count) => Instruction::Call(argument_count),
stage3c-publisher-tail-method-to-method-call|ordinary-leaf-consumer-lowering|src/runtime/binary_object_publish.rs|            Instruction::TailCallMethod(argument_count)|            Instruction::CallMethod(argument_count)
stage3a-publisher-array-from-wrong-op|ordinary-leaf-consumer-lowering|src/runtime/binary_object_publish.rs|        OrdinaryLeafOp::ArrayFrom(element_count) => Instruction::ArrayFrom(element_count),|        OrdinaryLeafOp::ArrayFrom(element_count) => Instruction::Construct(element_count),
stage3b-publisher-apply-kind-swap|ordinary-leaf-consumer-lowering|src/runtime/binary_object_publish.rs|            OrdinaryLeafApplyKind::Call => ApplyKind::Call,|            OrdinaryLeafApplyKind::Call => ApplyKind::Construct,
ordinary-synthetic-index-offset|ordinary-leaf-consumer-lowering|src/runtime/binary_object_publish.rs|            Instruction::PushConst(index)|            Instruction::PushConst(index.saturating_add(1))
ordinary-synthetic-bigint-coercion|ordinary-leaf-consumer-publication|src/runtime/binary_object_publish.rs|                    Value::BigInt(JsBigInt::from(*value)),|                    Value::BigInt(JsBigInt::from(value.unsigned_abs())),
TRANSLATE_CANARIES
expect_full_rewrite_rejected translate-ready-remap \
    function-translate-semantic-dispatch src/runtime/binary_object/function_translate/mod.rs \
    '    let ready = |operation| Ok(PendingExpansion::one(PendingOperation::Ready(operation)));' \
    '    let ready = |_operation| Ok(PendingExpansion::one(PendingOperation::Ready(FunctionOp::PushNull)));'
expect_full_rewrite_rejected translate-push-i32-payload \
    function-translate-semantic-dispatch src/runtime/binary_object/function_translate/mod.rs \
    $'        (Recipe::PushI32, NativeOperands::I32(value) | NativeOperands::NoneInt(value)) => {\n            ready(FunctionOp::PushI32(*value))\n        }' \
    $'        (Recipe::PushI32, NativeOperands::I32(value) | NativeOperands::NoneInt(value)) => {\n            ready(FunctionOp::PushI32(-*value))\n        }'
expect_full_rewrite_rejected translate-get-local-payload \
    function-translate-semantic-dispatch src/runtime/binary_object/function_translate/mod.rs \
    $'        (Recipe::GetLocal, NativeOperands::Loc(index) | NativeOperands::NoneLoc(index)) => {\n            ready(FunctionOp::GetLocal(*index))\n        }' \
    $'        (Recipe::GetLocal, NativeOperands::Loc(index) | NativeOperands::NoneLoc(index)) => {\n            ready(FunctionOp::GetLocal(index.saturating_add(1)))\n        }'
expect_full_rewrite_rejected translate-call-format-union-collapse \
    function-translate-semantic-dispatch src/runtime/binary_object/function_translate/mod.rs \
    $'        (\n            Recipe::Call,\n            NativeOperands::NPop(argument_count) | NativeOperands::NPopX(argument_count),\n        ) => ready(FunctionOp::Call(*argument_count)),' \
    $'        (Recipe::Call, NativeOperands::NPop(argument_count)) =>\n            ready(FunctionOp::Call(*argument_count)),'
expect_full_rewrite_rejected translate-call-argc-plus-one \
    function-translate-semantic-dispatch src/runtime/binary_object/function_translate/mod.rs \
    '        ) => ready(FunctionOp::Call(*argument_count)),' \
    '        ) => ready(FunctionOp::Call(argument_count.saturating_add(1))),'
expect_full_rewrite_rejected translate-call-argc-minus-one \
    function-translate-semantic-dispatch src/runtime/binary_object/function_translate/mod.rs \
    '        ) => ready(FunctionOp::Call(*argument_count)),' \
    '        ) => ready(FunctionOp::Call(argument_count.saturating_sub(1))),'
expect_full_rewrite_rejected stage3a-translate-construct-argc-plus-one \
    function-translate-semantic-dispatch src/runtime/binary_object/function_translate/mod.rs \
    $'        (Recipe::Construct, NativeOperands::NPop(argument_count)) => {\n            ready(FunctionOp::Construct(*argument_count))\n        }' \
    $'        (Recipe::Construct, NativeOperands::NPop(argument_count)) => {\n            ready(FunctionOp::Construct(argument_count.saturating_add(1)))\n        }'
expect_full_rewrite_rejected stage3a-translate-construct-call-method-swap \
    function-translate-semantic-dispatch src/runtime/binary_object/function_translate/mod.rs \
    $'        (Recipe::Construct, NativeOperands::NPop(argument_count)) => {\n            ready(FunctionOp::Construct(*argument_count))\n        }\n        (Recipe::CallMethod, NativeOperands::NPop(argument_count)) => {\n            ready(FunctionOp::CallMethod(*argument_count))\n        }' \
    $'        (Recipe::Construct, NativeOperands::NPop(argument_count)) => {\n            ready(FunctionOp::CallMethod(*argument_count))\n        }\n        (Recipe::CallMethod, NativeOperands::NPop(argument_count)) => {\n            ready(FunctionOp::Construct(*argument_count))\n        }'
expect_full_rewrite_rejected stage3c-translate-tail-call-to-call \
    function-translate-semantic-dispatch src/runtime/binary_object/function_translate/mod.rs \
    $'        (Recipe::TailCall, NativeOperands::NPop(argument_count)) => {\n            ready(FunctionOp::TailCall(*argument_count))\n        }' \
    $'        (Recipe::TailCall, NativeOperands::NPop(argument_count)) => {\n            ready(FunctionOp::Call(*argument_count))\n        }'
expect_full_rewrite_rejected stage3c-translate-tail-method-to-call-return \
    function-translate-semantic-dispatch src/runtime/binary_object/function_translate/mod.rs \
    $'        (Recipe::TailCallMethod, NativeOperands::NPop(argument_count)) => {\n            ready(FunctionOp::TailCallMethod(*argument_count))\n        }' \
    $'        (Recipe::TailCallMethod, NativeOperands::NPop(argument_count)) => {\n            Ok(PendingExpansion::two(\n                PendingOperation::Ready(FunctionOp::CallMethod(*argument_count)),\n                PendingOperation::Ready(FunctionOp::Return),\n            ))\n        }'
expect_full_rewrite_rejected stage3b-translate-apply-zero-kind \
    function-translate-semantic-dispatch src/runtime/binary_object/function_translate/mod.rs \
    '            ready(FunctionOp::Apply(FunctionApplyKind::Call))' \
    '            ready(FunctionOp::Apply(FunctionApplyKind::Construct))'
expect_full_rewrite_rejected stage3b-translate-apply-noncanonical-admission \
    function-translate-semantic-dispatch src/runtime/binary_object/function_translate/mod.rs \
    '            Err(FunctionTranslateError::non_canonical_apply_magic(*magic))' \
    '            ready(FunctionOp::Apply(FunctionApplyKind::Call))'
expect_full_rewrite_rejected stage3b-apply-error-classification \
    ordinary-leaf-apply-admission src/runtime/binary_object/ordinary_leaf.rs \
    '    if error.is_unadmitted_operand_error() {' \
    '    if false && error.is_unadmitted_operand_error() {'
expect_full_rewrite_rejected stage3b-apply-stack-effect \
    stage3b-apply-stack src/bytecode.rs \
    '            Self::Apply(_) | Self::ApplySuper => (3, 1),' \
    '            Self::Apply(_) | Self::ApplySuper => (2, 1),'
expect_full_rewrite_rejected stage3b-nullish-apply-bypass \
    stage3b-apply-order src/runtime/vm_host.rs \
    '        if matches!(argument_array, Value::Undefined | Value::Null) {' \
    '        if false && matches!(argument_array, Value::Undefined | Value::Null) {'
expect_full_rewrite_rejected stage3b-raw-new-target-collapse \
    stage3b-raw-construction src/runtime.rs \
    '            ConstructNewTarget::Raw(new_target) => {' \
    '            ConstructNewTarget::Validated(new_target) => {'
expect_full_rewrite_rejected stage3b-constructor-callable-narrowing \
    stage3b-constructor-capability src/runtime.rs \
    '            object_data.is_constructor' \
    '            object_data.is_constructor && object_data.is_callable'
expect_full_rewrite_rejected stage3b-bound-new-target-retarget \
    stage3b-raw-construction src/runtime.rs \
    '                    new_target.retarget_bound_identity(&constructor, &target);' \
    '                    let _ = (&new_target, &constructor, &target);'
expect_full_rewrite_rejected stage3b-proxy-before-callable \
    stage3b-raw-construction src/runtime.rs \
    '            if self.is_proxy_object(constructor.as_object())? {' \
    '            if false && self.is_proxy_object(constructor.as_object())? {'
expect_full_rewrite_rejected stage3b-function-realm-fallback \
    stage3b-constructor-prototype src/runtime.rs \
    '        self.function_realm_from_value(caller_realm, new_target)' \
    '        Ok(NativeConversion::Value(caller_realm))'
expect_full_rewrite_rejected stage3b-native-prototype-helper-bypass \
    stage3b-native-prototype-family src/runtime/intrinsics/array_buffer.rs \
    '        self.prototype_from_constructor_value(realm, &new_target, |fallback_realm| {' \
    '        self.constructor_prototype_source(realm, &new_target).map(|_| |fallback_realm| {'
expect_full_rewrite_rejected stage3b-proxy-call-layer-capability \
    stage3b-proxy-call-order src/runtime/internal_methods.rs \
    '            if !rooted.data.is_callable {' \
    '            if false && !rooted.data.is_callable {'
expect_full_rewrite_rejected stage3b-proxy-construct-callable-narrowing \
    stage3b-proxy-construct-order src/runtime/internal_methods.rs \
    '                match self.constructor_from_value(realm, Value::Object(rooted.target.clone()))? {' \
    '                match self.callable_from_value(Value::Object(rooted.target.clone())) {'
expect_full_rewrite_rejected stage3b-public-raw-construction-leak \
    stage3b-public-construction src/runtime/context.rs \
    '            .construct_internal(self.realm, constructor, new_target, arguments)' \
    '            .construct_value_with_raw_new_target_internal(self.realm, Value::Object(constructor.as_object().clone()), Value::Object(new_target.as_object().clone()), arguments)'
expect_full_rewrite_rejected stage3b-species-callable-narrowing \
    stage3b-species-constructor src/runtime/intrinsics/promise.rs \
    '    ) -> Result<NativeConversion<Option<ConstructorRef>>, RuntimeError> {' \
    '    ) -> Result<NativeConversion<Option<CallableRef>>, RuntimeError> {'
expect_full_rewrite_table <<'STAGE3B_PAYLOAD_CANARIES'
stage3b-noncanonical-error-constructor|function-translate-apply-admission|src/runtime/binary_object/function_translate/mod.rs|            kind: FunctionTranslateErrorKind::NonCanonicalApplyMagic(magic),|            kind: FunctionTranslateErrorKind::AllocationFailed,
stage3b-noncanonical-error-classifier|function-translate-apply-admission|src/runtime/binary_object/function_translate/mod.rs|    pub(in crate::runtime::binary_object) const fn is_unadmitted_operand_error(&self) -> bool {\n        matches!(|    pub(in crate::runtime::binary_object) const fn is_unadmitted_operand_error(&self) -> bool {\n        false && matches!(
stage3b-apply-call-payload|stage3b-apply-order|src/runtime/vm_host.rs|            ApplyKind::Call => self\n                .runtime\n                .call_internal(\n                    self.current_realm,\n                    &callable,\n                    this_or_new_target,\n                    &arguments,\n                )|            ApplyKind::Call => self\n                .runtime\n                .call_internal(\n                    self.current_realm,\n                    &callable,\n                    Value::Undefined,\n                    &arguments,\n                )
stage3b-apply-construct-payload|stage3b-apply-order|src/runtime/vm_host.rs|                    this_or_new_target,\n                    &arguments,\n                )\n                .map_err(runtime_error_to_vm_error),\n        }|                    Value::Undefined,\n                    &[],\n                )\n                .map_err(runtime_error_to_vm_error),\n        }
stage3b-raw-wrapper-payload|stage3b-raw-construction|src/runtime.rs|            ConstructNewTarget::Raw(new_target),|            ConstructNewTarget::Raw(Value::Undefined),
stage3b-validated-wrapper-payload|stage3b-validated-construction|src/runtime.rs|            ConstructNewTarget::Validated(new_target.clone()),|            ConstructNewTarget::Validated(constructor.clone()),
stage3b-wrapper-arguments|stage3b-raw-construction|src/runtime.rs|            ConstructNewTarget::Raw(new_target),\n            arguments,|            ConstructNewTarget::Raw(new_target),\n            &[],
stage3b-native-new-target-payload|stage3b-raw-construction|src/runtime.rs|                        min_readable_args,\n                        new_target.value(),\n                        &arguments,|                        min_readable_args,\n                        Value::Undefined,\n                        &arguments,
stage3b-base-new-target-shadow|stage3b-raw-construction|src/runtime.rs|                    let raw_new_target = new_target.value();|                    let raw_new_target = new_target.value();\n                    let raw_new_target = Value::Undefined;
stage3b-prototype-get-receiver|stage3b-constructor-prototype|src/runtime.rs|            caller_realm,\n            new_target.clone(),\n            &prototype_key,|            caller_realm,\n            Value::Undefined,\n            &prototype_key,
stage3b-prototype-realm-map|stage3b-constructor-prototype|src/runtime.rs|                NativeConversion::Value(realm) => {\n                    NativeConversion::Value(ConstructorPrototypeSource::Realm(realm))|                NativeConversion::Value(_realm) => {\n                    NativeConversion::Value(ConstructorPrototypeSource::Realm(caller_realm))
stage3b-function-realm-bound-progress|stage3b-function-realm|src/runtime/internal_methods.rs|                ObjectPayload::BoundFunction { target, .. } => {\n                    let target = *target;\n                    drop(state);\n                    object = ObjectRef::from_borrowed_handle(self.clone(), target)?;|                ObjectPayload::BoundFunction { target, .. } => {\n                    let target = *target;\n                    drop(state);\n                    let _ = target;\n                    object = object.clone();
stage3b-proxy-call-progress|stage3b-proxy-call-order|src/runtime/internal_methods.rs|                if self.is_proxy_object(&rooted.target)? {\n                    current = rooted.target.clone();|                if self.is_proxy_object(&rooted.target)? {\n                    current = current.clone();
stage3b-proxy-construct-progress|stage3b-proxy-construct-order|src/runtime/internal_methods.rs|                    current = target;|                    current = current.clone();
stage3b-proxy-call-fallback-payload|stage3b-proxy-call-order|src/runtime/internal_methods.rs|                    Value::Object(rooted.target.clone()),\n                    this_value,\n                    arguments,|                    Value::Object(rooted.target.clone()),\n                    Value::Undefined,\n                    &[],
stage3b-proxy-call-trap-payload|stage3b-proxy-call-order|src/runtime/internal_methods.rs|                    Value::Object(rooted.target.clone()),\n                    this_value,\n                    Value::Object(argument_array),|                    Value::Object(rooted.handler.clone()),\n                    Value::Undefined,\n                    Value::Object(argument_array),
stage3b-proxy-construct-trap-payload|stage3b-proxy-construct-order|src/runtime/internal_methods.rs|                    Value::Object(rooted.target.clone()),\n                    Value::Object(argument_array),\n                    new_target.value(),|                    Value::Object(rooted.handler.clone()),\n                    Value::Object(argument_array),\n                    Value::Undefined,
stage3b-array-from-capability|stage3b-species-constructor|src/runtime/intrinsics/array.rs|            let arguments = length.into_iter().collect::<Vec<_>>();|            let _ = self.callable_from_value(Value::Object(constructor.as_object().clone()))?;\n            let arguments = length.into_iter().collect::<Vec<_>>();
stage3b-array-of-capability|stage3b-species-constructor|src/runtime/intrinsics/array.rs|            match self.construct_constructor_internal(\n                realm,\n                &constructor,|            let _ = self.callable_from_value(Value::Object(constructor.as_object().clone()))?;\n            match self.construct_constructor_internal(\n                realm,\n                &constructor,
stage3b-array-species-capability|stage3b-species-constructor|src/runtime/intrinsics/array.rs|        self.construct_constructor_internal(\n            realm,\n            &constructor,\n            &constructor,\n            &[Value::number(length as f64)],|        let _ = self.callable_from_value(Value::Object(constructor.as_object().clone()))?;\n        self.construct_constructor_internal(\n            realm,\n            &constructor,\n            &constructor,\n            &[Value::number(length as f64)],
stage3b-species-return-payload|stage3b-species-constructor|src/runtime/intrinsics/promise.rs|                NativeConversion::Value(constructor) => NativeConversion::Value(Some(constructor)),|                NativeConversion::Value(_constructor) => NativeConversion::Value(None),
stage3b-typed-species-arguments|stage3b-species-constructor|src/runtime/intrinsics/array_buffer/typed_array/species.rs|            &constructor,\n            &constructor,\n            arguments,|            &constructor,\n            &constructor,\n            &[],
stage3b-runtime-evidence-not-ignored|stage3b-runtime-evidence|src/runtime/tests.rs|#[test]\nfn trusted_quickjs_ordinary_apply_verifies_stack_and_reindexed_branch_targets() {|#[test]\n#[ignore = "gate mutation"]\nfn trusted_quickjs_ordinary_apply_verifies_stack_and_reindexed_branch_targets() {
stage3b-runtime-evidence-prefix-not-ignored|stage3b-runtime-evidence|src/runtime/tests.rs|#[test]\nfn trusted_quickjs_ordinary_apply_verifies_stack_and_reindexed_branch_targets() {|#[ignore = "gate mutation"]\n#[test]\nfn trusted_quickjs_ordinary_apply_verifies_stack_and_reindexed_branch_targets() {
stage3b-runtime-evidence-cfg-not-ignored|stage3b-runtime-evidence|src/runtime/tests.rs|#[test]\nfn trusted_quickjs_ordinary_apply_verifies_stack_and_reindexed_branch_targets() {|#[cfg_attr(test, ignore)]\n#[test]\nfn trusted_quickjs_ordinary_apply_verifies_stack_and_reindexed_branch_targets() {
stage3b-runtime-evidence-not-cfg-excluded|stage3b-runtime-evidence|src/runtime/tests.rs|#[test]\nfn trusted_quickjs_ordinary_apply_verifies_stack_and_reindexed_branch_targets() {|#[cfg(any())]\n#[test]\nfn trusted_quickjs_ordinary_apply_verifies_stack_and_reindexed_branch_targets() {
STAGE3B_PAYLOAD_CANARIES
expect_full_rewrite_rejected stage3b-apply-nullish-prework \
    stage3b-apply-order src/runtime/vm_host.rs \
    $'        if matches!(argument_array, Value::Undefined | Value::Null) {\n            return self' \
    $'        if matches!(argument_array, Value::Undefined | Value::Null) {\n            let _ = self.build_argument_list(Value::Undefined)?;\n            return self'
expect_full_rewrite_rejected stage3b-native-prototype-payload \
    stage3b-native-prototype-family src/runtime/intrinsics/array_buffer.rs \
    '        self.prototype_from_constructor_value(realm, &new_target, |fallback_realm| {' \
    '        self.prototype_from_constructor_value(realm, &Value::Undefined, |fallback_realm| {'
expect_full_rewrite_table <<'STAGE3C_CANARIES'
stage3c-instruction-tail-payload|stage3c-instruction-shape|src/bytecode.rs|    TailCall(u16),|    TailCall(u32),
stage3c-tail-call-stack-effect|stage3c-tail-verifier|src/bytecode.rs|            Self::TailCall(argument_count) => (*argument_count as usize + 1, 0),|            Self::TailCall(argument_count) => (*argument_count as usize + 1, 1),
stage3c-tail-method-stack-effect|stage3c-tail-verifier|src/bytecode.rs|            Self::TailCallMethod(argument_count) => (*argument_count as usize + 2, 0),|            Self::TailCallMethod(argument_count) => (*argument_count as usize + 1, 0),
stage3c-tail-plain-receiver|stage3c-tail-vm|src/vm.rs|                return host.call(function, Value::Undefined, arguments).map(Some);|                return host.call(function, Value::Null, arguments).map(Some);
stage3c-tail-method-pop-order|stage3c-tail-vm|src/vm.rs|                let function = self.pop()?;\n                let receiver = self.pop()?;\n                return host.call(function, receiver, arguments).map(Some);|                let receiver = self.pop()?;\n                let function = self.pop()?;\n                return host.call(function, receiver, arguments).map(Some);
stage3c-tail-early-frame-clear|stage3c-tail-vm|src/vm.rs|                let function = self.pop()?;\n                return host.call(function, Value::Undefined, arguments).map(Some);|                let function = self.pop()?;\n                self.stack.clear();\n                return host.call(function, Value::Undefined, arguments).map(Some);
stage3c-tail-return-not-terminal|stage3c-tail-completion|src/vm.rs|                Ok(InterpreterExit::Complete(Completion::Return(value))) => {\n                    return Ok(Completion::Return(value));\n                }\n                Ok(InterpreterExit::Complete(Completion::Throw(value))) => value,|                Ok(InterpreterExit::Complete(Completion::Return(value))) => value,\n                Ok(InterpreterExit::Complete(Completion::Throw(value))) => value,
stage3c-tail-throw-skips-backtrace|stage3c-tail-completion|src/vm.rs|        host.ensure_backtrace(&value)?;\n        loop {|        loop {
stage3c-runtime-evidence-not-ignored|stage3c-runtime-evidence|src/runtime/tests.rs|#[test]\nfn trusted_quickjs_ordinary_tail_invocations_use_exact_bc5_wires_and_semantics() {|#[test]\n#[ignore = "gate mutation"]\nfn trusted_quickjs_ordinary_tail_invocations_use_exact_bc5_wires_and_semantics() {
stage3c-vm-evidence-not-cfg-excluded|stage3c-runtime-evidence|src/vm.rs|    #[test]\n    fn tail_invocation_throws_use_the_activation_backtrace_and_catch_path() {|    #[cfg(any())]\n    #[test]\n    fn tail_invocation_throws_use_the_activation_backtrace_and_catch_path() {
stage3c-translate-evidence-not-cfg-ignored|stage3c-runtime-evidence|src/runtime/binary_object/function_translate/mod.rs|    #[test]\n    fn tail_invocation_lowering_preserves_the_npop_operand_and_kind() {|    #[cfg_attr(test, ignore)]\n    #[test]\n    fn tail_invocation_lowering_preserves_the_npop_operand_and_kind() {
STAGE3C_CANARIES
expect_full_rewrite_rejected stage3c-tail-terminal-fallthrough \
    stage3c-tail-verifier src/bytecode.rs \
    $'            Instruction::TailCall(_)\n            | Instruction::TailCallMethod(_)\n            | Instruction::Return\n            | Instruction::ReturnUndefined\n            | Instruction::ReturnDerived(_)\n            | Instruction::Throw\n            | Instruction::Ret => {}' \
    $'            Instruction::Return\n            | Instruction::ReturnUndefined\n            | Instruction::ReturnDerived(_)\n            | Instruction::Throw\n            | Instruction::Ret => {}'
expect_full_rewrite_rejected stage3c-blocker-mapping-swap \
    function-translate-registry-blockers \
    src/runtime/binary_object/function_translate/capability.rs \
    $'    row!(3, Const, Blocked, FunctionGraph),\n    row!(4, Atom, ScalarOnly, Recipe::PushAtom),\n    row!(5, Atom, Blocked, ValueConstruction),' \
    $'    row!(3, Const, Blocked, ValueConstruction),\n    row!(4, Atom, ScalarOnly, Recipe::PushAtom),\n    row!(5, Atom, Blocked, FunctionGraph),'
expect_full_rewrite_rejected stage3c-translate-ready-shadow \
    function-translate-semantic-dispatch \
    src/runtime/binary_object/function_translate/mod.rs \
    $'    let ready = |operation| Ok(PendingExpansion::one(PendingOperation::Ready(operation)));\n    match (recipe, operands) {' \
    $'    let ready = |operation| Ok(PendingExpansion::one(PendingOperation::Ready(operation)));\n    let ready = |operation| {\n        let operation = match operation {\n            FunctionOp::TailCall(argument_count) => FunctionOp::Call(argument_count),\n            FunctionOp::TailCallMethod(argument_count) => FunctionOp::CallMethod(argument_count),\n            operation => operation,\n        };\n        Ok(PendingExpansion::one(PendingOperation::Ready(operation)))\n    };\n    match (recipe, operands) {'
expect_full_rewrite_rejected stage3c-ordinary-guarded-tail-bypass \
    ordinary-leaf-translated-code \
    src/runtime/binary_object/ordinary_leaf.rs \
    $'    match operation {\n        FunctionOp::Nop => Ok(OrdinaryLeafOp::Nop),\n        FunctionOp::Object => Ok(OrdinaryLeafOp::Object),\n        FunctionOp::ToObject => Ok(OrdinaryLeafOp::ToObject),\n        FunctionOp::PushThis => Ok(OrdinaryLeafOp::PushThis),\n        FunctionOp::PushI32(value) => Ok(OrdinaryLeafOp::PushI32(*value)),' \
    $'    if matches!(operation, FunctionOp::TailCall(0)) {\n        return Ok(OrdinaryLeafOp::Call(0));\n    }\n    match operation {\n        FunctionOp::Nop => Ok(OrdinaryLeafOp::Nop),\n        FunctionOp::Object => Ok(OrdinaryLeafOp::Object),\n        FunctionOp::ToObject => Ok(OrdinaryLeafOp::ToObject),\n        FunctionOp::PushThis => Ok(OrdinaryLeafOp::PushThis),\n        FunctionOp::PushI32(value) => Ok(OrdinaryLeafOp::PushI32(*value)),'
expect_full_rewrite_rejected stage3c-publisher-alias-tail-bypass \
    ordinary-leaf-consumer-lowering \
    src/runtime/binary_object_publish.rs \
    $'    let instruction = match operation {\n        OrdinaryLeafOp::Nop => Instruction::Nop,\n        OrdinaryLeafOp::Object => Instruction::Object,\n        OrdinaryLeafOp::ToObject => Instruction::ToObject,\n        OrdinaryLeafOp::PushThis => Instruction::PushThis,\n        OrdinaryLeafOp::PushI32(value) => Instruction::PushI32(value),' \
    $'    use OrdinaryLeafOp as O;\n    if let O::TailCall(argument_count) = &operation {\n        return Ok(Instruction::Call(*argument_count));\n    }\n    let instruction = match operation {\n        OrdinaryLeafOp::Nop => Instruction::Nop,\n        OrdinaryLeafOp::Object => Instruction::Object,\n        OrdinaryLeafOp::ToObject => Instruction::ToObject,\n        OrdinaryLeafOp::PushThis => Instruction::PushThis,\n        OrdinaryLeafOp::PushI32(value) => Instruction::PushI32(value),'
expect_full_rewrite_rejected stage3c-stack-effect-guarded-bypass \
    stage3c-tail-verifier src/bytecode.rs \
    $'    pub const fn stack_effect(&self) -> (usize, usize) {\n        match self {' \
    $'    pub const fn stack_effect(&self) -> (usize, usize) {\n        if let Self::TailCall(0) | Self::TailCallMethod(0) = self {\n            return (1, 1);\n        }\n        match self {'
expect_full_rewrite_rejected stage3c-verifier-alias-fallthrough \
    stage3c-tail-verifier src/bytecode.rs \
    $'        record_maximum_depth(&mut maximum, next_depth, declared_max_stack)?;\n        // QuickJS `compute_stack_size` stops as soon as a reachable PC crosses' \
    $'        record_maximum_depth(&mut maximum, next_depth, declared_max_stack)?;\n        use Instruction as I;\n        if let I::TailCall(_) | I::TailCallMethod(_) = instruction {\n            enqueue_fallthrough(\n                &mut worklist,\n                pc,\n                VerificationState {\n                    depth: next_depth,\n                    regions: next_regions.clone(),\n                    return_addresses: next_return_addresses.clone(),\n                    super_call_bases: next_super_call_bases.clone(),\n                },\n                code.len(),\n            )?;\n        }\n        // QuickJS `compute_stack_size` stops as soon as a reachable PC crosses'
expect_full_rewrite_rejected stage3c-call-arguments-shadow \
    stage3c-tail-vm src/vm.rs \
    $'    ) -> Result<Vec<Value>, Error> {\n        let argument_count = usize::from(argument_count);' \
    $'    ) -> Result<Vec<Value>, Error> {\n        let argument_count = 0;\n        let argument_count = usize::from(argument_count);'
expect_full_rewrite_rejected stage3c-call-arguments-drop \
    stage3c-tail-vm src/vm.rs \
    $'        let start = self.stack.len() - argument_count;\n        Ok(self.stack.split_off(start))' \
    $'        let start = self.stack.len() - argument_count;\n        let mut arguments = self.stack.split_off(start);\n        arguments.pop();\n        Ok(arguments)'
expect_full_rewrite_rejected stage3c-call-dispatch-alias-bypass \
    stage3c-tail-vm src/vm.rs \
    $'    ) -> Result<Option<Completion>, Error> {\n        let completion = match instruction {\n            Instruction::Import => {' \
    $'    ) -> Result<Option<Completion>, Error> {\n        use Instruction as I;\n        let completion = match instruction {\n            I::TailCall(argument_count) if *argument_count == 0 => {\n                let _ = self.pop()?;\n                return host.call(Value::Undefined, Value::Null, Vec::new()).map(Some);\n            }\n            Instruction::Import => {'
expect_full_rewrite_rejected stage3c-execute-inner-tail-intercept \
    stage3c-tail-vm src/vm.rs \
    $'            if matches!(\n                instruction,\n                Instruction::Import\n                    | Instruction::Call(_)' \
    $'            use Instruction as I;\n            if matches!(instruction, I::TailCall(_) | I::TailCallMethod(_)) {\n                return Ok(InterpreterExit::Complete(Completion::Return(Value::Undefined)));\n            }\n\n            if matches!(\n                instruction,\n                Instruction::Import\n                    | Instruction::Call(_)'
expect_full_rewrite_rejected stage3c-execute-alias-return-bypass \
    stage3c-tail-completion src/vm.rs \
    $'    ) -> Result<Completion, Error> {\n        loop {\n            let raised = match self.execute_inner(code, host) {' \
    $'    ) -> Result<Completion, Error> {\n        use Completion as C;\n        loop {\n            let raised = match self.execute_inner(code, host) {\n                Ok(InterpreterExit::Complete(C::Return(value))) if matches!(&value, Value::Undefined) => {\n                    self.pc = self.pc.saturating_add(1);\n                    continue;\n                }'
expect_full_rewrite_rejected stage3c-run-throw-bypass \
    stage3c-tail-completion src/vm.rs \
    $'                Ok(InterpreterExit::Complete(Completion::Return(value))) => {\n                    return Ok(VmExit::Complete(Completion::Return(value)));\n                }\n                Ok(InterpreterExit::Complete(Completion::Throw(value))) => value,' \
    $'                Ok(InterpreterExit::Complete(Completion::Return(value))) => {\n                    return Ok(VmExit::Complete(Completion::Return(value)));\n                }\n                Ok(InterpreterExit::Complete(Completion::Throw(value)))\n                    if matches!(&value, Value::Undefined) => {\n                        return Ok(VmExit::Complete(Completion::Throw(value)));\n                    }\n                Ok(InterpreterExit::Complete(Completion::Throw(value))) => value,'
expect_full_rewrite_rejected stage3c-raise-guarded-bypass \
    stage3c-tail-completion src/vm.rs \
    $'    ) -> Result<Option<Completion>, Error> {\n        host.ensure_backtrace(&value)?;\n        loop {' \
    $'    ) -> Result<Option<Completion>, Error> {\n        if matches!(value, Value::Undefined) {\n            return Ok(Some(Completion::Throw(value)));\n        }\n        host.ensure_backtrace(&value)?;\n        loop {'
expect_full_rewrite_rejected stage3c-required-module-cfg-excluded \
    stage3c-runtime-evidence src/vm.rs \
    $'#[cfg(test)]\nmod tests {' \
    $'#[cfg(any())]\n#[cfg(test)]\nmod tests {'
expect_full_rewrite_rejected stage3c-required-module-inner-cfg-excluded \
    stage3c-runtime-evidence src/runtime/tests.rs \
    'use crate::JsBigInt;' \
    $'#![cfg(any())]\n\nuse crate::JsBigInt;'
expect_full_rewrite_rejected stage3c-required-test-macro-shadow \
    stage3c-runtime-evidence src/runtime/tests.rs \
    $'fn trusted_quickjs_ordinary_tail_invocations_use_exact_bc5_wires_and_semantics() {\n    assert_eq!(QUICKJS_ORDINARY_TAIL_CALL_BC5.len(), 57);' \
    $'fn trusted_quickjs_ordinary_tail_invocations_use_exact_bc5_wires_and_semantics() {\n    macro_rules! assert_eq { ($($tokens:tt)*) => {}; }\n    assert_eq!(QUICKJS_ORDINARY_TAIL_CALL_BC5.len(), 57);'
expect_full_rewrite_table <<'STAGE3D_CANARIES'
stage3d-raw48-shared|function-translate-registry-audience|src/runtime/binary_object/function_translate/capability.rs|    row!(48, None, OrdinaryOnly, Recipe::Throw),|    row!(48, None, Shared, Recipe::Throw),
stage3d-raw47-alias-admission|function-translate-registry-audience|src/runtime/binary_object/function_translate/capability.rs|    row!(47, None, Blocked, Completion),|    row!(47, None, OrdinaryOnly, Recipe::Throw),
stage3d-translate-throw-to-return|function-translate-semantic-dispatch|src/runtime/binary_object/function_translate/mod.rs|        (Recipe::Throw, NativeOperands::None) => ready(FunctionOp::Throw),|        (Recipe::Throw, NativeOperands::None) => ready(FunctionOp::Return),
stage3d-ordinary-throw-to-return|ordinary-leaf-translated-code|src/runtime/binary_object/ordinary_leaf.rs|        FunctionOp::Throw => Ok(OrdinaryLeafOp::Throw),|        FunctionOp::Throw => Ok(OrdinaryLeafOp::Return),
stage3d-publisher-throw-to-return|ordinary-leaf-consumer-lowering|src/runtime/binary_object_publish.rs|        OrdinaryLeafOp::Throw => Instruction::Throw,|        OrdinaryLeafOp::Throw => Instruction::Return,
stage3d-runtime-evidence-ignored|stage3d-runtime-evidence|src/runtime/tests.rs|#[test]\nfn trusted_quickjs_ordinary_throw_uses_the_exact_wire_metadata_and_value_identity() {|#[test]\n#[ignore = "gate mutation"]\nfn trusted_quickjs_ordinary_throw_uses_the_exact_wire_metadata_and_value_identity() {
stage3d-runtime-evidence-cfg-excluded|stage3d-runtime-evidence|src/runtime/tests.rs|#[test]\nfn trusted_quickjs_ordinary_throw_reenters_caller_catch_backtrace_and_iterator_close() {|#[cfg(any())]\n#[test]\nfn trusted_quickjs_ordinary_throw_reenters_caller_catch_backtrace_and_iterator_close() {
stage3d-runtime-evidence-early-return|stage3d-runtime-evidence|src/runtime/tests.rs|fn trusted_quickjs_ordinary_throw_is_terminal_and_branch_targetable() {\n    let runtime = Runtime::new();|fn trusted_quickjs_ordinary_throw_is_terminal_and_branch_targetable() {\n    return;\n    let runtime = Runtime::new();
stage3d-c-oracle-disabled|stage3d-c-oracle|tests/fixtures/function_bytecode_wire.c|    if (expect_ordinary_throw_completion(compile_context))|    if (0 && expect_ordinary_throw_completion(compile_context))
STAGE3D_CANARIES
expect_full_rewrite_table <<'STAGE3E_CANARIES'
stage3e-raw49-shared|function-translate-registry-audience|src/runtime/binary_object/function_translate/capability.rs|    row!(49, AtomU8, OrdinaryOnly, Recipe::ThrowReadOnly),|    row!(49, AtomU8, Shared, Recipe::ThrowReadOnly),
stage3e-raw49-throw-alias|function-translate-registry-policy|src/runtime/binary_object/function_translate/capability.rs|    row!(49, AtomU8, OrdinaryOnly, Recipe::ThrowReadOnly),|    row!(49, AtomU8, OrdinaryOnly, Recipe::Throw),
stage3e-subtype-one-admission|function-translate-semantic-dispatch|src/runtime/binary_object/function_translate/mod.rs|        (Recipe::ThrowReadOnly, NativeOperands::AtomU8 { atom, value: 0 }) => {|        (Recipe::ThrowReadOnly, NativeOperands::AtomU8 { atom, value: 1 }) => {
stage3e-ordinary-read-only-to-throw|ordinary-leaf-translated-code|src/runtime/binary_object/ordinary_leaf.rs|        FunctionOp::ThrowReadOnly(atom) => {\n            copy_read_only_name(atom).map(OrdinaryLeafOp::ThrowReadOnly)\n        }|        FunctionOp::ThrowReadOnly(_) => Ok(OrdinaryLeafOp::Throw),
stage3e-two-input-atoms-admitted|stage3e-read-only-atom-ledger|src/runtime/binary_object/ordinary_leaf.rs|        if declared_slots > 1 {|        if declared_slots > 2 {
stage3e-unused-input-atom-admitted|stage3e-read-only-atom-ledger|src/runtime/binary_object/ordinary_leaf.rs|        if self.declared_slots == 1 && !self.used_input_slot {|        if self.declared_slots == 1 && false {
stage3e-non-string-atom-admitted|stage3e-read-only-atom-ledger|src/runtime/binary_object/ordinary_leaf.rs|    if atom.class() != AtomOperandClass::String {|    if atom.class() == AtomOperandClass::String {
stage3e-synthetic-name-not-string|stage3e-read-only-publication|src/runtime/binary_object_publish.rs|                    constants.push(lower_primitive_constant(Value::String(value))?);|                    constants.push(lower_primitive_constant(Value::Undefined)?);
stage3e-publisher-index-forced-zero|ordinary-leaf-consumer-lowering|src/runtime/binary_object_publish.rs|            Instruction::ThrowReadOnly(index)|            Instruction::ThrowReadOnly(0)
stage3e-runtime-wire-subtype-alias|stage3e-runtime-evidence|src/runtime/tests.rs|    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x06, 0x00, 0x31, 0xf3, 0x00, 0x00, 0x00, 0x00,|    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x06, 0x00, 0x31, 0xf3, 0x00, 0x00, 0x00, 0x01,
stage3e-runtime-type-error-test-ignored|stage3e-runtime-evidence|src/runtime/tests.rs|#[test]\nfn trusted_quickjs_ordinary_read_only_uses_exact_zero_stack_wire_and_type_error() {|#[test]\n#[ignore = "gate mutation"]\nfn trusted_quickjs_ordinary_read_only_uses_exact_zero_stack_wire_and_type_error() {
stage3e-runtime-catch-test-cfg-excluded|stage3e-runtime-evidence|src/runtime/tests.rs|#[test]\nfn trusted_quickjs_ordinary_read_only_reenters_catch_and_resets_pending_state() {|#[cfg(any())]\n#[test]\nfn trusted_quickjs_ordinary_read_only_reenters_catch_and_resets_pending_state() {
stage3e-runtime-realm-test-early-return|stage3e-runtime-evidence|src/runtime/tests.rs|fn trusted_quickjs_ordinary_read_only_uses_bytecode_realm_and_attaches_backtrace() {\n    let runtime = Runtime::new();|fn trusted_quickjs_ordinary_read_only_uses_bytecode_realm_and_attaches_backtrace() {\n    return;\n    let runtime = Runtime::new();
stage3e-c-oracle-disabled|stage3d-c-oracle|tests/fixtures/function_bytecode_wire.c|    if (expect_ordinary_throw_error_completion(compile_context))|    if (0 && expect_ordinary_throw_error_completion(compile_context))
stage3f-status-inherited-coverage-erased|stage3f-status|docs/status.md|retains the Stage-3H raw-111 `ToObject`, Stage-3G\nraw-11 Object, and Stage-3F raw-177 coverage|retains the Stage-3H raw-111 `ToObject` and Stage-3G\nraw-11 Object but drops the Stage-3F raw-177 coverage
STAGE3E_CANARIES
expect_full_rewrite_table <<'STAGE3F_CANARIES'
stage3f-raw177-shared|function-translate-registry-audience|src/runtime/binary_object/function_translate/capability.rs|    row!(177, None, OrdinaryOnly, Recipe::Nop),|    row!(177, None, Shared, Recipe::Nop),
stage3f-raw177-recipe-alias|function-translate-registry-policy|src/runtime/binary_object/function_translate/capability.rs|    row!(177, None, OrdinaryOnly, Recipe::Nop),|    row!(177, None, OrdinaryOnly, Recipe::ReturnUndefined),
stage3f-translate-nop-remap|function-translate-semantic-dispatch|src/runtime/binary_object/function_translate/mod.rs|        (Recipe::Nop, NativeOperands::None) => ready(FunctionOp::Nop),|        (Recipe::Nop, NativeOperands::None) => ready(FunctionOp::ReturnUndefined),
stage3f-ordinary-nop-remap|ordinary-leaf-translated-code|src/runtime/binary_object/ordinary_leaf.rs|        FunctionOp::Nop => Ok(OrdinaryLeafOp::Nop),|        FunctionOp::Nop => Ok(OrdinaryLeafOp::ReturnUndefined),
stage3f-publisher-nop-drop|ordinary-leaf-consumer-lowering|src/runtime/binary_object_publish.rs|        OrdinaryLeafOp::Nop => Instruction::Nop,|        OrdinaryLeafOp::Nop => Instruction::Drop,
stage3f-publisher-nop-push|ordinary-leaf-consumer-lowering|src/runtime/binary_object_publish.rs|        OrdinaryLeafOp::Nop => Instruction::Nop,|        OrdinaryLeafOp::Nop => Instruction::Undefined,
stage3f-publisher-nop-synthetic-index|ordinary-leaf-consumer-lowering|src/runtime/binary_object_publish.rs|        OrdinaryLeafOp::Nop => Instruction::Nop,|        OrdinaryLeafOp::Nop => { *next_synthetic_index += 1; Instruction::Nop },
stage3f-runtime-branch-index-collapse|stage3f-runtime-evidence|src/runtime/tests.rs|            Instruction::Goto(1),\n            Instruction::Nop,|            Instruction::Goto(0),\n            Instruction::Nop,
stage3f-vm-nop-push-effect|stage3f-nop-vm|src/vm.rs|            Instruction::Nop => {}|            Instruction::Nop => { self.stack.push(Value::Undefined); }
stage3f-wire-code-len-one|stage3f-runtime-evidence|src/runtime/tests.rs|    0x00, 0x00, 0x00, 0x00, 0x00, 0x02, 0x00, 0xb1, 0x29,|    0x00, 0x00, 0x00, 0x00, 0x00, 0x01, 0x00, 0xb1, 0x29,
stage3f-wire-raw177-aliased|stage3f-runtime-evidence|src/runtime/tests.rs|    0x00, 0x00, 0x00, 0x00, 0x00, 0x02, 0x00, 0xb1, 0x29,|    0x00, 0x00, 0x00, 0x00, 0x00, 0x02, 0x00, 0x29, 0x29,
stage3f-wire-terminal-removed|stage3f-runtime-evidence|src/runtime/tests.rs|    0x00, 0x00, 0x00, 0x00, 0x00, 0x02, 0x00, 0xb1, 0x29,|    0x00, 0x00, 0x00, 0x00, 0x00, 0x02, 0x00, 0xb1, 0xb1,
stage3f-wire-stack-one|stage3f-runtime-evidence|src/runtime/tests.rs|    0x00, 0x00, 0x00, 0x00, 0x00, 0x02, 0x00, 0xb1, 0x29,|    0x00, 0x00, 0x00, 0x01, 0x00, 0x02, 0x00, 0xb1, 0x29,
stage3f-runtime-wire-test-ignored|stage3f-runtime-evidence|src/runtime/tests.rs|#[test]\nfn trusted_quickjs_ordinary_nop_preserves_exact_metadata_realm_and_zero_effect() {|#[test]\n#[ignore = "gate mutation"]\nfn trusted_quickjs_ordinary_nop_preserves_exact_metadata_realm_and_zero_effect() {
stage3f-runtime-fallthrough-test-cfg-excluded|stage3f-runtime-evidence|src/runtime/tests.rs|#[test]\nfn trusted_quickjs_ordinary_nop_only_fallthrough_rolls_back_and_retries() {|#[cfg(any())]\n#[test]\nfn trusted_quickjs_ordinary_nop_only_fallthrough_rolls_back_and_retries() {
stage3f-runtime-branch-test-early-return|stage3f-runtime-evidence|src/runtime/tests.rs|fn trusted_quickjs_ordinary_branch_targets_raw177_typed_index() {\n    let mut image = QUICKJS_ORDINARY_NOP_BC5.to_vec();|fn trusted_quickjs_ordinary_branch_targets_raw177_typed_index() {\n    return;\n    let mut image = QUICKJS_ORDINARY_NOP_BC5.to_vec();
stage3f-c-oracle-disabled|stage3d-c-oracle|tests/fixtures/function_bytecode_wire.c|    if (expect_ordinary_nop_completion(compile_context))|    if (0 && expect_ordinary_nop_completion(compile_context))
stage3f-status-typed-chain-erased|stage3f-status|docs/status.md|`Recipe::Nop` to `FunctionOp::Nop` to `OrdinaryLeafOp::Nop` and finally the|`Recipe::Nop` directly to `Instruction::Nop`, bypassing the typed DTO chain, and finally the
stage3f-status-premature-source-current-hidden|stage3f-status|docs/status.md|The same oracle pins compatible 32-bit `scope_next` wrapping|<!-- Stage 3F is source-current and authenticated. -->\n\nThe same oracle pins compatible 32-bit `scope_next` wrapping
stage3f-status-no-longer-source-ahead|stage3f-status|docs/status.md|The same oracle pins compatible 32-bit `scope_next` wrapping|Stage 3F is no longer source-ahead.\n\nThe same oracle pins compatible 32-bit `scope_next` wrapping
stage3f-status-not-unauthenticated|stage3f-status|docs/status.md|The same oracle pins compatible 32-bit `scope_next` wrapping|Stage 3F is not unauthenticated.\n\nThe same oracle pins compatible 32-bit `scope_next` wrapping
stage3f-status-already-authenticated|stage3f-status|docs/status.md|The same oracle pins compatible 32-bit `scope_next` wrapping|Stage 3F is already authenticated.\n\nThe same oracle pins compatible 32-bit `scope_next` wrapping
stage3f-status-source-ahead-not-unauthenticated|stage3f-status|docs/status.md|The same oracle pins compatible 32-bit `scope_next` wrapping|Stage 3F is source-ahead but not unauthenticated.\n\nThe same oracle pins compatible 32-bit `scope_next` wrapping
stage3f-status-receipt-authenticates-raw177|stage3f-status|docs/status.md|The same oracle pins compatible 32-bit `scope_next` wrapping|This receipt authenticates raw177.\n\nThe same oracle pins compatible 32-bit `scope_next` wrapping
stage3f-status-artifact-certifies-raw177|stage3f-status|docs/status.md|The same oracle pins compatible 32-bit `scope_next` wrapping|This artifact certifies raw177.\n\nThe same oracle pins compatible 32-bit `scope_next` wrapping
stage3f-status-presently-current|stage3f-status|docs/status.md|The same oracle pins compatible 32-bit `scope_next` wrapping|Stage 3F is presently current.\n\nThe same oracle pins compatible 32-bit `scope_next` wrapping
stage3f-status-became-authenticated|stage3f-status|docs/status.md|The same oracle pins compatible 32-bit `scope_next` wrapping|Stage 3F became authenticated.\n\nThe same oracle pins compatible 32-bit `scope_next` wrapping
stage3f-status-now-counts-covered|stage3f-status|docs/status.md|The same oracle pins compatible 32-bit `scope_next` wrapping|Stage 3F now counts as covered.\n\nThe same oracle pins compatible 32-bit `scope_next` wrapping
stage3f-status-markdown-strong-stage-alias|stage3f-status|docs/status.md|The same oracle pins compatible 32-bit `scope_next` wrapping|Stage **3F** is authenticated.\n\nThe same oracle pins compatible 32-bit `scope_next` wrapping
stage3f-status-html-em-stage-alias|stage3f-status|docs/status.md|The same oracle pins compatible 32-bit `scope_next` wrapping|Stage <em>3F</em> is authenticated.\n\nThe same oracle pins compatible 32-bit `scope_next` wrapping
stage3f-status-comment-split-stage-alias|stage3f-status|docs/status.md|The same oracle pins compatible 32-bit `scope_next` wrapping|Stage<!-- boundary -->3F is authenticated.\n\nThe same oracle pins compatible 32-bit `scope_next` wrapping
stage3f-status-numeric-entity-stage-alias|stage3f-status|docs/status.md|The same oracle pins compatible 32-bit `scope_next` wrapping|Stage&#32;3F is authenticated.\n\nThe same oracle pins compatible 32-bit `scope_next` wrapping
stage3f-status-nbsp-stage-alias|stage3f-status|docs/status.md|The same oracle pins compatible 32-bit `scope_next` wrapping|Stage&nbsp;3F is authenticated.\n\nThe same oracle pins compatible 32-bit `scope_next` wrapping
stage3f-status-markdown-strong-raw177-alias|stage3f-status|docs/status.md|The same oracle pins compatible 32-bit `scope_next` wrapping|This receipt authenticates raw **177**.\n\nThe same oracle pins compatible 32-bit `scope_next` wrapping
stage3f-status-html-span-raw177-alias|stage3f-status|docs/status.md|The same oracle pins compatible 32-bit `scope_next` wrapping|This artifact certifies raw<span>177</span>.\n\nThe same oracle pins compatible 32-bit `scope_next` wrapping
stage3f-status-comment-split-raw177-alias|stage3f-status|docs/status.md|The same oracle pins compatible 32-bit `scope_next` wrapping|This run covers raw<!-- boundary -->177.\n\nThe same oracle pins compatible 32-bit `scope_next` wrapping
stage3f-status-code-span-stage-alias|stage3f-status|docs/status.md|The same oracle pins compatible 32-bit `scope_next` wrapping|Stage `3F` is authenticated.\n\nThe same oracle pins compatible 32-bit `scope_next` wrapping
stage3f-status-code-span-raw177-alias|stage3f-status|docs/status.md|The same oracle pins compatible 32-bit `scope_next` wrapping|This receipt authenticates raw`177`.\n\nThe same oracle pins compatible 32-bit `scope_next` wrapping
stage3f-status-entity-code-span-stage-alias|stage3f-status|docs/status.md|The same oracle pins compatible 32-bit `scope_next` wrapping|Stage &#96;3F&#96; is authenticated.\n\nThe same oracle pins compatible 32-bit `scope_next` wrapping
stage3f-status-inline-link-stage-alias|stage3f-status|docs/status.md|The same oracle pins compatible 32-bit `scope_next` wrapping|Stage [3F](#x) is authenticated.\n\nThe same oracle pins compatible 32-bit `scope_next` wrapping
stage3f-status-inline-link-raw177-alias|stage3f-status|docs/status.md|The same oracle pins compatible 32-bit `scope_next` wrapping|This receipt authenticates raw[177](#x).\n\nThe same oracle pins compatible 32-bit `scope_next` wrapping
stage3f-status-reference-link-stage-alias|stage3f-status|docs/status.md|The same oracle pins compatible 32-bit `scope_next` wrapping|Stage [3F][x] is authenticated.\n\n[x]: #x\n\nThe same oracle pins compatible 32-bit `scope_next` wrapping
stage3f-status-reference-link-raw177-alias|stage3f-status|docs/status.md|The same oracle pins compatible 32-bit `scope_next` wrapping|This receipt authenticates raw[177][x].\n\n[x]: #x\n\nThe same oracle pins compatible 32-bit `scope_next` wrapping
stage3f-status-strikethrough-stage-alias|stage3f-status|docs/status.md|The same oracle pins compatible 32-bit `scope_next` wrapping|Stage ~~3F~~ is authenticated.\n\nThe same oracle pins compatible 32-bit `scope_next` wrapping
stage3f-status-strikethrough-raw177-alias|stage3f-status|docs/status.md|The same oracle pins compatible 32-bit `scope_next` wrapping|This receipt authenticates raw~~177~~.\n\nThe same oracle pins compatible 32-bit `scope_next` wrapping
stage3f-status-zwsp-stage-alias|stage3f-status|docs/status.md|The same oracle pins compatible 32-bit `scope_next` wrapping|Stage\342\200\2133F is authenticated.\n\nThe same oracle pins compatible 32-bit `scope_next` wrapping
stage3f-status-word-joiner-stage-alias|stage3f-status|docs/status.md|The same oracle pins compatible 32-bit `scope_next` wrapping|Stage\342\201\2403F is authenticated.\n\nThe same oracle pins compatible 32-bit `scope_next` wrapping
stage3f-status-soft-hyphen-stage-alias|stage3f-status|docs/status.md|The same oracle pins compatible 32-bit `scope_next` wrapping|Stage\302\2553F is authenticated.\n\nThe same oracle pins compatible 32-bit `scope_next` wrapping
stage3f-status-zwsp-raw177-alias|stage3f-status|docs/status.md|The same oracle pins compatible 32-bit `scope_next` wrapping|This receipt authenticates raw\342\200\213177.\n\nThe same oracle pins compatible 32-bit `scope_next` wrapping
stage3f-status-numeric-zwsp-stage-alias|stage3f-status|docs/status.md|The same oracle pins compatible 32-bit `scope_next` wrapping|Stage&#8203;3F is authenticated.\n\nThe same oracle pins compatible 32-bit `scope_next` wrapping
stage3f-status-comment-markdown-stage-alias|stage3f-status|docs/status.md|The same oracle pins compatible 32-bit `scope_next` wrapping|<!-- Stage **3F** is authenticated. -->\n\nThe same oracle pins compatible 32-bit `scope_next` wrapping
stage3f-status-comment-markdown-raw177-alias|stage3f-status|docs/status.md|The same oracle pins compatible 32-bit `scope_next` wrapping|<!-- This receipt authenticates raw **177**. -->\n\nThe same oracle pins compatible 32-bit `scope_next` wrapping
stage3f-status-fullwidth-stage-alias|stage3f-status|docs/status.md|The same oracle pins compatible 32-bit `scope_next` wrapping|Ｓｔａｇｅ ３Ｆ is authenticated.\n\nThe same oracle pins compatible 32-bit `scope_next` wrapping
stage3f-status-fullwidth-token-stage-alias|stage3f-status|docs/status.md|The same oracle pins compatible 32-bit `scope_next` wrapping|Stage ３Ｆ is authenticated.\n\nThe same oracle pins compatible 32-bit `scope_next` wrapping
stage3f-status-superscript-script-stage-alias|stage3f-status|docs/status.md|The same oracle pins compatible 32-bit `scope_next` wrapping|Stage ³ℱ is authenticated.\n\nThe same oracle pins compatible 32-bit `scope_next` wrapping
stage3f-status-math-bold-stage-alias|stage3f-status|docs/status.md|The same oracle pins compatible 32-bit `scope_next` wrapping|𝐒𝐭𝐚𝐠𝐞 𝟑𝐅 is authenticated.\n\nThe same oracle pins compatible 32-bit `scope_next` wrapping
stage3f-status-circled-stage-alias|stage3f-status|docs/status.md|The same oracle pins compatible 32-bit `scope_next` wrapping|Ⓢⓣⓐⓖⓔ ③Ⓕ is authenticated.\n\nThe same oracle pins compatible 32-bit `scope_next` wrapping
stage3f-status-fullwidth-raw177-alias|stage3f-status|docs/status.md|The same oracle pins compatible 32-bit `scope_next` wrapping|This receipt authenticates ｒａｗ１７７.\n\nThe same oracle pins compatible 32-bit `scope_next` wrapping
stage3f-status-math-bold-raw177-alias|stage3f-status|docs/status.md|The same oracle pins compatible 32-bit `scope_next` wrapping|This receipt authenticates 𝐫𝐚𝐰𝟏𝟕𝟕.\n\nThe same oracle pins compatible 32-bit `scope_next` wrapping
stage3f-status-markdown-image-stage-alt|stage3f-status|docs/status.md|The same oracle pins compatible 32-bit `scope_next` wrapping|Stage ![3F](missing.png) is authenticated.\n\nThe same oracle pins compatible 32-bit `scope_next` wrapping
stage3f-status-markdown-image-raw177-alt|stage3f-status|docs/status.md|The same oracle pins compatible 32-bit `scope_next` wrapping|This receipt authenticates raw![177](missing.png).\n\nThe same oracle pins compatible 32-bit `scope_next` wrapping
stage3f-status-nested-linked-image-stage-alt|stage3f-status|docs/status.md|The same oracle pins compatible 32-bit `scope_next` wrapping|Stage [![3F](missing.png)](#x) is authenticated.\n\nThe same oracle pins compatible 32-bit `scope_next` wrapping
stage3f-status-html-img-stage-alt|stage3f-status|docs/status.md|The same oracle pins compatible 32-bit `scope_next` wrapping|Stage <img src="missing.png" alt="3F"> is authenticated.\n\nThe same oracle pins compatible 32-bit `scope_next` wrapping
stage3f-status-html-img-raw177-alt|stage3f-status|docs/status.md|The same oracle pins compatible 32-bit `scope_next` wrapping|This receipt authenticates raw<img src="missing.png" alt="177">.\n\nThe same oracle pins compatible 32-bit `scope_next` wrapping
stage3f-status-shortcut-image-stage-alt|stage3f-status|docs/status.md|The same oracle pins compatible 32-bit `scope_next` wrapping|Stage ![3F] is authenticated.\n\n[3F]: missing.png\n\nThe same oracle pins compatible 32-bit `scope_next` wrapping
stage3f-status-html-img-decoy-title-real-stage-alt|stage3f-status|docs/status.md|The same oracle pins compatible 32-bit `scope_next` wrapping|Stage <img title="decoy alt='x'" src="missing.png" alt="3F"> is authenticated.\n\nThe same oracle pins compatible 32-bit `scope_next` wrapping
STAGE3F_CANARIES
expect_full_rewrite_table <<'STAGE3G_CANARIES'
stage3g-raw11-shared|function-translate-registry-audience|src/runtime/binary_object/function_translate/capability.rs|    row!(11, None, OrdinaryOnly, Recipe::Object),|    row!(11, None, Shared, Recipe::Object),
stage3g-raw11-recipe-alias|function-translate-registry-policy|src/runtime/binary_object/function_translate/capability.rs|    row!(11, None, OrdinaryOnly, Recipe::Object),|    row!(11, None, OrdinaryOnly, Recipe::Nop),
stage3g-raw47-object-alias|function-translate-registry-audience|src/runtime/binary_object/function_translate/capability.rs|    row!(47, None, Blocked, Completion),|    row!(47, None, OrdinaryOnly, Recipe::Object),
stage3g-recipe-object-payload|function-translate-recipe-shape|src/runtime/binary_object/function_translate/capability.rs|    Nop,\n    Object,\n    ToObject,\n    PushThis,\n    PushI32,|    Nop,\n    Object(u8),\n    ToObject,\n    PushThis,\n    PushI32,
stage3g-function-object-erased|function-translate-dto-shape|src/runtime/binary_object/function_translate/dto.rs|    OutsideTarget,\n    Nop,\n    Object,\n    ToObject,\n    PushThis,\n    PushI32(i32),|    OutsideTarget,\n    Nop,\n    ToObject,\n    PushThis,\n    PushI32(i32),
stage3g-translate-object-remap|stage3g-object-translation-route|src/runtime/binary_object/function_translate/mod.rs|        (Recipe::Object, NativeOperands::None) => ready(FunctionOp::Object),|        (Recipe::Object, NativeOperands::None) => ready(FunctionOp::Nop),
stage3g-ordinary-object-remap|stage3g-object-ordinary-route|src/runtime/binary_object/ordinary_leaf.rs|        FunctionOp::Object => Ok(OrdinaryLeafOp::Object),|        FunctionOp::Object => Ok(OrdinaryLeafOp::Nop),
stage3g-publisher-object-wrong|stage3g-object-publication|src/runtime/binary_object_publish.rs|        OrdinaryLeafOp::Object => Instruction::Object,|        OrdinaryLeafOp::Object => Instruction::Undefined,
stage3g-publisher-object-drop|stage3g-object-publication|src/runtime/binary_object_publish.rs|        OrdinaryLeafOp::Object => Instruction::Object,|        OrdinaryLeafOp::Object => Instruction::Nop,
stage3g-publisher-object-synthetic-effect|stage3g-object-publication|src/runtime/binary_object_publish.rs|        OrdinaryLeafOp::Object => Instruction::Object,|        OrdinaryLeafOp::Object => { *next_synthetic_index += 1; Instruction::Object },
stage3g-object-stack-effect-zero|stage3g-object-verifier|src/bytecode.rs|            Self::Object => (0, 1),|            Self::Object => (0, 0),
stage3g-vm-object-drop|stage3g-object-vm|src/vm.rs|            Instruction::Object => match host.object()? {\n                Completion::Return(object) => self.stack.push(object),|            Instruction::Object => match host.object()? {\n                Completion::Return(_object) => {},
stage3g-vm-object-wrong-value|stage3g-object-vm|src/vm.rs|            Instruction::Object => match host.object()? {\n                Completion::Return(object) => self.stack.push(object),|            Instruction::Object => match host.object()? {\n                Completion::Return(_object) => self.stack.push(Value::Undefined),
stage3g-host-object-wrong-realm|stage3g-object-realm|src/runtime/vm_host.rs|            .new_ordinary_object_in_realm(self.current_realm)|            .new_ordinary_object_in_realm(ContextId::ROOT)
stage3g-wire-raw11-aliased|stage3g-runtime-evidence|src/runtime/tests.rs|    0x00, 0x01, 0x00, 0x00, 0x00, 0x02, 0x00, 0x0b, 0x28,|    0x00, 0x01, 0x00, 0x00, 0x00, 0x02, 0x00, 0xb1, 0x28,
stage3g-runtime-max-stack-erased|stage3g-runtime-evidence|src/runtime/tests.rs|        [Instruction::Object, Instruction::Return]\n    ));\n    assert!(snapshot.constants.is_empty());\n    assert_eq!(snapshot.metadata.argument_count, 0);\n    assert_eq!(snapshot.metadata.defined_argument_count, 0);\n    assert_eq!(snapshot.metadata.local_count, 0);\n    assert_eq!(snapshot.metadata.max_stack, 1);|        [Instruction::Object, Instruction::Return]\n    ));\n    assert!(snapshot.constants.is_empty());\n    assert_eq!(snapshot.metadata.argument_count, 0);\n    assert_eq!(snapshot.metadata.defined_argument_count, 0);\n    assert_eq!(snapshot.metadata.local_count, 0);\n    assert_eq!(snapshot.metadata.max_stack, 0);
stage3g-runtime-fallthrough-negative-erased|stage3g-runtime-evidence|src/runtime/tests.rs|    object_only[37] = 1;\n    object_only.truncate(40);|    object_only[37] = 1;\n    object_only.truncate(41);
stage3g-runtime-branch-index-collapse|stage3g-runtime-evidence|src/runtime/tests.rs|            Instruction::Goto(1),\n            Instruction::Object,|            Instruction::Goto(0),\n            Instruction::Object,
stage3g-runtime-freshness-erased|stage3g-runtime-evidence|src/runtime/tests.rs|            assert_ne!(object, other, "raw11 reused an Object allocation");|            assert_eq!(object, other, "raw11 reused an Object allocation");
stage3g-runtime-wire-test-cfg-excluded|stage3g-runtime-evidence|src/runtime/tests.rs|#[test]\nfn trusted_quickjs_ordinary_object_is_natural_fresh_and_defining_realm_owned() {|#[cfg(any())]\n#[test]\nfn trusted_quickjs_ordinary_object_is_natural_fresh_and_defining_realm_owned() {
stage3g-c-oracle-disabled|stage3d-c-oracle|tests/fixtures/function_bytecode_wire.c|    if (expect_ordinary_object_completion(compile_context))|    if (0 && expect_ordinary_object_completion(compile_context))
stage3g-c-transcript-max-stack-erased|stage3g-c-oracle|tests/fixtures/function_bytecode_wire.quickjs-2026-06-04.txt|ordinary-object-child-metadata=flags:0243,js_mode:1,args:0,vars:0,defined_args:0,stack:1,var_refs:0,closures:0,cpool:0,code:2,locals:0,code_offset:39,atoms:0|ordinary-object-child-metadata=flags:0243,js_mode:1,args:0,vars:0,defined_args:0,stack:0,var_refs:0,closures:0,cpool:0,code:2,locals:0,code_offset:39,atoms:0
stage3g-status-inherited-coverage-erased|stage3g-status|docs/status.md|retains the Stage-3H raw-111 `ToObject`, Stage-3G\nraw-11 Object, and Stage-3F raw-177 coverage|retains the Stage-3H raw-111 `ToObject` and Stage-3F\nraw-177 coverage
STAGE3G_CANARIES
expect_full_rewrite_rejected stage3g-translate-object-erased \
    stage3g-object-translation-route \
    src/runtime/binary_object/function_translate/mod.rs \
    '                PendingOperation::Ready(operation) => operation,' \
    $'                PendingOperation::Ready(FunctionOp::Object) => continue,\n                PendingOperation::Ready(operation) => operation,'
expect_full_rewrite_rejected stage3g-translate-object-alias-erased \
    stage3g-object-translation-route \
    src/runtime/binary_object/function_translate/mod.rs \
    '                PendingOperation::Ready(operation) => operation,' \
    $'                PendingOperation::Ready(operation) => {\n                    use FunctionOp as O;\n                    if matches!(operation, O::Object) {\n                        continue;\n                    }\n                    operation\n                },'
expect_full_rewrite_rejected stage3g-object-verifier-terminal-bypass \
    stage3g-object-verifier src/bytecode.rs \
    $'            Instruction::ThrowReadOnly(_)\n            | Instruction::ThrowRedeclaration(_)' \
    $'            Instruction::Object\n            | Instruction::ThrowReadOnly(_)\n            | Instruction::ThrowRedeclaration(_)'
expect_full_rewrite_rejected stage3g-runtime-test-macro-shadow \
    stage3e-runtime-evidence src/runtime/tests.rs \
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
expect_full_rewrite_table <<'STAGE3H_CANARIES'
stage3h-raw111-shared|function-translate-registry-audience|src/runtime/binary_object/function_translate/capability.rs|    row!(111, None, OrdinaryOnly, Recipe::ToObject),|    row!(111, None, Shared, Recipe::ToObject),
stage3h-raw111-recipe-alias|function-translate-registry-policy|src/runtime/binary_object/function_translate/capability.rs|    row!(111, None, OrdinaryOnly, Recipe::ToObject),|    row!(111, None, OrdinaryOnly, Recipe::Object),
stage3h-raw47-to-object-alias|function-translate-registry-audience|src/runtime/binary_object/function_translate/capability.rs|    row!(47, None, Blocked, Completion),|    row!(47, None, OrdinaryOnly, Recipe::ToObject),
stage3h-raw112-to-object-alias|function-translate-registry-audience|src/runtime/binary_object/function_translate/capability.rs|    row!(112, None, Blocked, ValueConstruction),|    row!(112, None, OrdinaryOnly, Recipe::ToObject),
stage3h-recipe-to-object-payload|function-translate-recipe-shape|src/runtime/binary_object/function_translate/capability.rs|    Object,\n    ToObject,\n    PushThis,\n    PushI32,|    Object,\n    ToObject(u8),\n    PushThis,\n    PushI32,
stage3h-function-to-object-erased|function-translate-dto-shape|src/runtime/binary_object/function_translate/dto.rs|    Object,\n    ToObject,\n    PushThis,\n    PushI32(i32),|    Object,\n    PushThis,\n    PushI32(i32),
stage3h-ordinary-to-object-erased|ordinary-leaf-operation-shape|src/runtime/binary_object/ordinary_leaf.rs|    Object,\n    ToObject,\n    PushThis,\n    PushI32(i32),|    Object,\n    PushThis,\n    PushI32(i32),
stage3h-translate-to-object-remap|stage3h-to-object-translation-route|src/runtime/binary_object/function_translate/mod.rs|        (Recipe::ToObject, NativeOperands::None) => ready(FunctionOp::ToObject),|        (Recipe::ToObject, NativeOperands::None) => ready(FunctionOp::Object),
stage3h-ordinary-to-object-remap|stage3h-to-object-ordinary-route|src/runtime/binary_object/ordinary_leaf.rs|        FunctionOp::ToObject => Ok(OrdinaryLeafOp::ToObject),|        FunctionOp::ToObject => Ok(OrdinaryLeafOp::Object),
stage3h-publisher-to-object-wrong|stage3h-to-object-publication|src/runtime/binary_object_publish.rs|        OrdinaryLeafOp::ToObject => Instruction::ToObject,|        OrdinaryLeafOp::ToObject => Instruction::Object,
stage3h-publisher-to-object-synthetic-effect|stage3h-to-object-publication|src/runtime/binary_object_publish.rs|        OrdinaryLeafOp::ToObject => Instruction::ToObject,|        OrdinaryLeafOp::ToObject => { *next_synthetic_index += 1; Instruction::ToObject },
stage3h-vm-to-object-identity-drift|stage3h-to-object-vm|src/vm.rs|                    value @ Value::Object(_) => self.stack.push(value),|                    Value::Object(_) => self.stack.push(Value::Undefined),
stage3h-vm-to-object-coercion|stage3h-to-object-vm|src/vm.rs|                    primitive => self.stack.push(host.box_primitive(primitive)?),|                    primitive => self.stack.push(host.box_primitive(host.to_primitive(primitive, ToPrimitiveHint::Default)?.into_value())?),
stage3h-host-to-object-realm-drift|stage3h-to-object-realm|src/runtime/vm_host.rs|            Value::Bool(_) => (\n                PrimitiveKind::Boolean,\n                self.runtime\n                    .primitive_prototype_for_realm(self.current_realm, PrimitiveKind::Boolean)|            Value::Bool(_) => (\n                PrimitiveKind::Boolean,\n                self.runtime\n                    .primitive_prototype_for_realm(ContextId::ROOT, PrimitiveKind::Boolean)
stage3h-manual-wire-byte-drift|stage3h-runtime-evidence|src/runtime/tests.rs|    0x01, 0x01, 0x00, 0x00, 0x00, 0x03, 0x01, 0x00, 0x01, 0x00, 0x00, 0xcf, 0x6f, 0x28,|    0x01, 0x01, 0x00, 0x00, 0x00, 0x03, 0x01, 0x00, 0x01, 0x00, 0x00, 0xcf, 0x70, 0x28,
stage3h-runtime-max-stack-erased|stage3h-runtime-evidence|src/runtime/tests.rs|            Instruction::GetArg(0),\n            Instruction::ToObject,\n            Instruction::Return,\n        ]\n    ));\n    assert!(snapshot.constants.is_empty());\n    assert_eq!(snapshot.metadata.argument_count, 1);\n    assert_eq!(snapshot.metadata.defined_argument_count, 1);\n    assert_eq!(snapshot.metadata.local_count, 0);\n    assert_eq!(snapshot.metadata.max_stack, 1);|            Instruction::GetArg(0),\n            Instruction::ToObject,\n            Instruction::Return,\n        ]\n    ));\n    assert!(snapshot.constants.is_empty());\n    assert_eq!(snapshot.metadata.argument_count, 1);\n    assert_eq!(snapshot.metadata.defined_argument_count, 1);\n    assert_eq!(snapshot.metadata.local_count, 0);\n    assert_eq!(snapshot.metadata.max_stack, 0);
stage3h-runtime-fallthrough-negative-erased|stage3h-runtime-evidence|src/runtime/tests.rs|    fallthrough[37] = 2;\n    fallthrough.truncate(45);|    fallthrough[37] = 3;\n    fallthrough.truncate(46);
stage3h-runtime-max-stack-negative-erased|stage3h-runtime-evidence|src/runtime/tests.rs|    let mut undersized_stack = QUICKJS_ORDINARY_TO_OBJECT_BC5.to_vec();\n    undersized_stack[33] = 0;|    let mut undersized_stack = QUICKJS_ORDINARY_TO_OBJECT_BC5.to_vec();\n    undersized_stack[33] = 1;
stage3h-runtime-branch-index-collapse|stage3h-runtime-evidence|src/runtime/tests.rs|            Instruction::Goto(2),\n            Instruction::ToObject,|            Instruction::Goto(1),\n            Instruction::ToObject,
stage3h-runtime-natural-test-cfg-excluded|stage3h-runtime-evidence|src/runtime/tests.rs|#[test]\nfn trusted_quickjs_ordinary_to_object_is_natural_exact_and_realm_correct() {|#[cfg(any())]\n#[test]\nfn trusted_quickjs_ordinary_to_object_is_natural_exact_and_realm_correct() {
stage3h-runtime-nullish-test-cfg-excluded|stage3h-runtime-evidence|src/runtime/tests.rs|#[test]\nfn trusted_quickjs_ordinary_to_object_nullish_is_pending_and_catchable() {|#[cfg(any())]\n#[test]\nfn trusted_quickjs_ordinary_to_object_nullish_is_pending_and_catchable() {
stage3h-c-oracle-disabled|stage3d-c-oracle|tests/fixtures/function_bytecode_wire.c|    if (expect_ordinary_to_object_completion(compile_context))|    if (0 && expect_ordinary_to_object_completion(compile_context))
stage3h-c-mechanical-wire-remap|stage3d-c-oracle|tests/fixtures/function_bytecode_wire.c|    manual_wire[44] = 111;|    manual_wire[44] = 112;
stage3h-c-transcript-provenance-concealed|stage3h-c-oracle|tests/fixtures/function_bytecode_wire.quickjs-2026-06-04.txt|ordinary-to-object-evidence=compiler-natural-provenance-plus-mechanically-derived-property-free-wire|ordinary-to-object-evidence=two-compiler-natural-wires
stage3h-c-manifest-source-hash-drift|stage3i-c-oracle|dev-support/quickjs-c-oracles.tsv|e6d93033db5e00b403ab203e598bc66f77d079329680e970d779d63a388ff0c4|e6d93033db5e00b403ab203e598bc66f77d079329680e970d779d63a388ff0c5
stage3h-status-c-provenance-erased|stage3h-status|docs/status.md|The oracle honestly labels this as compiler\nprovenance, then mechanically changes|The oracle labels both wires as compiler\nprovenance, then changes
stage3h-status-inherited-coverage-erased|stage3h-status|docs/status.md|retains the Stage-3H raw-111 `ToObject`, Stage-3G\nraw-11 Object, and Stage-3F raw-177 coverage|retains the Stage-3G\nraw-11 Object and Stage-3F raw-177 coverage
stage3i-status-stale-claim|stage3i-status|docs/status.md|The same oracle pins compatible 32-bit `scope_next` wrapping|Stage 3I is source-stale and unauthenticated.\n\nThe same oracle pins compatible 32-bit `scope_next` wrapping
stage3i-status-not-current-claim|stage3i-status|docs/status.md|The same oracle pins compatible 32-bit `scope_next` wrapping|Stage 3I is not source-current.\n\nThe same oracle pins compatible 32-bit `scope_next` wrapping
stage3j-status-source-current-claim|stage3j-status|docs/status.md|The same oracle pins compatible 32-bit `scope_next` wrapping|Stage 3J is source-current and authenticated.\n\nThe same oracle pins compatible 32-bit `scope_next` wrapping
stage3j-status-receipt-covers-claim|stage3j-status|docs/status.md|The same oracle pins compatible 32-bit `scope_next` wrapping|This promoted receipt covers raw112 and Stage 3J.\n\nThe same oracle pins compatible 32-bit `scope_next` wrapping
stage3j-status-cyrillic-je-current-claim|stage3j-status|docs/status.md|The same oracle pins compatible 32-bit `scope_next` wrapping|Stage 3Ј is source-current and authenticated.\n\nThe same oracle pins compatible 32-bit `scope_next` wrapping
stage3j-status-fullwidth-j-current-claim|stage3j-status|docs/status.md|The same oracle pins compatible 32-bit `scope_next` wrapping|Stage 3Ｊ is source-current and authenticated.\n\nThe same oracle pins compatible 32-bit `scope_next` wrapping
stage3j-status-control-split-current-claim|stage3j-status|docs/status.md|The same oracle pins compatible 32-bit `scope_next` wrapping|Stage 3\001J is source-current and authenticated.\n\nThe same oracle pins compatible 32-bit `scope_next` wrapping
stage3h-status-emphasis-stale-claim|stage3h-status|docs/status.md|The same oracle pins compatible 32-bit `scope_next` wrapping|Stage 3*H* is source-stale and unauthenticated.\n\nThe same oracle pins compatible 32-bit `scope_next` wrapping
stage3h-status-strong-stale-claim|stage3h-status|docs/status.md|The same oracle pins compatible 32-bit `scope_next` wrapping|Stage 3**H** is source-stale and unauthenticated.\n\nThe same oracle pins compatible 32-bit `scope_next` wrapping
stage3h-status-strong-emphasis-stale-claim|stage3h-status|docs/status.md|The same oracle pins compatible 32-bit `scope_next` wrapping|Stage 3***H*** is source-stale and unauthenticated.\n\nThe same oracle pins compatible 32-bit `scope_next` wrapping
stage3i-status-emphasis-stale-claim|stage3i-status|docs/status.md|The same oracle pins compatible 32-bit `scope_next` wrapping|Stage 3*I* is source-stale and unauthenticated.\n\nThe same oracle pins compatible 32-bit `scope_next` wrapping
stage3i-status-strong-stale-claim|stage3i-status|docs/status.md|The same oracle pins compatible 32-bit `scope_next` wrapping|Stage 3**I** is source-stale and unauthenticated.\n\nThe same oracle pins compatible 32-bit `scope_next` wrapping
stage3i-status-strong-emphasis-stale-claim|stage3i-status|docs/status.md|The same oracle pins compatible 32-bit `scope_next` wrapping|Stage 3***I*** is source-stale and unauthenticated.\n\nThe same oracle pins compatible 32-bit `scope_next` wrapping
stage3j-status-emphasis-current-claim|stage3j-status|docs/status.md|The same oracle pins compatible 32-bit `scope_next` wrapping|Stage 3*J* is source-current and authenticated.\n\nThe same oracle pins compatible 32-bit `scope_next` wrapping
stage3j-status-strong-current-claim|stage3j-status|docs/status.md|The same oracle pins compatible 32-bit `scope_next` wrapping|Stage 3**J** is source-current and authenticated.\n\nThe same oracle pins compatible 32-bit `scope_next` wrapping
stage3j-status-strong-emphasis-current-claim|stage3j-status|docs/status.md|The same oracle pins compatible 32-bit `scope_next` wrapping|Stage 3***J*** is source-current and authenticated.\n\nThe same oracle pins compatible 32-bit `scope_next` wrapping
STAGE3H_CANARIES
expect_full_rewrite_table <<'STAGE3I_CANARIES'
stage3i-raw8-shared|function-translate-registry-audience|src/runtime/binary_object/function_translate/capability.rs|    row!(8, None, OrdinaryOnly, Recipe::PushThis),|    row!(8, None, Shared, Recipe::PushThis),
stage3i-raw8-recipe-alias|function-translate-registry-policy|src/runtime/binary_object/function_translate/capability.rs|    row!(8, None, OrdinaryOnly, Recipe::PushThis),|    row!(8, None, OrdinaryOnly, Recipe::ToObject),
stage3i-raw47-admitted|function-translate-registry-audience|src/runtime/binary_object/function_translate/capability.rs|    row!(47, None, Blocked, Completion),|    row!(47, None, OrdinaryOnly, Recipe::PushThis),
stage3i-raw112-admitted|function-translate-registry-audience|src/runtime/binary_object/function_translate/capability.rs|    row!(112, None, Blocked, ValueConstruction),|    row!(112, None, OrdinaryOnly, Recipe::PushThis),
stage3i-recipe-push-this-payload|function-translate-recipe-shape|src/runtime/binary_object/function_translate/capability.rs|    Object,\n    ToObject,\n    PushThis,\n    PushI32,|    Object,\n    ToObject,\n    PushThis(u8),\n    PushI32,
stage3i-function-push-this-erased|function-translate-dto-shape|src/runtime/binary_object/function_translate/dto.rs|    Object,\n    ToObject,\n    PushThis,\n    PushI32(i32),|    Object,\n    ToObject,\n    PushI32(i32),
stage3i-ordinary-push-this-erased|ordinary-leaf-operation-shape|src/runtime/binary_object/ordinary_leaf.rs|    Object,\n    ToObject,\n    PushThis,\n    PushI32(i32),|    Object,\n    ToObject,\n    PushI32(i32),
stage3i-translate-push-this-remap|stage3i-push-this-translation-route|src/runtime/binary_object/function_translate/mod.rs|        (Recipe::PushThis, NativeOperands::None) => ready(FunctionOp::PushThis),|        (Recipe::PushThis, NativeOperands::None) => ready(FunctionOp::ToObject),
stage3i-ordinary-push-this-remap|stage3i-push-this-ordinary-route|src/runtime/binary_object/ordinary_leaf.rs|        FunctionOp::PushThis => Ok(OrdinaryLeafOp::PushThis),|        FunctionOp::PushThis => Ok(OrdinaryLeafOp::ToObject),
stage3i-publisher-push-this-remap|stage3i-push-this-publication|src/runtime/binary_object_publish.rs|        OrdinaryLeafOp::PushThis => Instruction::PushThis,|        OrdinaryLeafOp::PushThis => Instruction::Nop,
stage3i-publisher-push-this-synthetic|stage3i-push-this-publication|src/runtime/binary_object_publish.rs|        OrdinaryLeafOp::PushThis => Instruction::PushThis,|        OrdinaryLeafOp::PushThis => { *next_synthetic_index += 1; Instruction::PushThis },
stage3i-validator-direct-bypass|stage3i-push-this-protocol|src/runtime/binary_object/ordinary_leaf.rs|    validate_push_this_protocol(code)?;|    let _ = validate_push_this_protocol;
stage3i-validator-alias-bypass|stage3i-push-this-protocol|src/runtime/binary_object/ordinary_leaf.rs|    validate_push_this_protocol(code)?;|    let validate = validate_push_this_protocol;\n    let _ = validate;
stage3i-validator-helper-bypass|stage3i-push-this-protocol|src/runtime/binary_object/ordinary_leaf.rs|    validate_push_this_protocol(code)?;|    fn accept_push_this(_: &FunctionCode<'_>) -> Result<(), OrdinaryLeafReadError> { Ok(()) }\n    accept_push_this(code)?;
stage3i-validator-pre-match-bypass|stage3i-push-this-protocol|src/runtime/binary_object/ordinary_leaf.rs|    validate_push_this_protocol(code)?;|    if code.instructions().first().is_some() { return Ok(Vec::new().into_boxed_slice()); }\n    validate_push_this_protocol(code)?;
stage3i-validator-match-drift|stage3i-push-this-protocol|src/runtime/binary_object/ordinary_leaf.rs|        if matches!(instruction.operation(), FunctionOp::PushThis) {|        if matches!(instruction.operation(), FunctionOp::Nop) {
stage3i-validator-absent-drift|stage3i-push-this-protocol|src/runtime/binary_object/ordinary_leaf.rs|    if push_this_count == 0 {|    if push_this_count == usize::MAX {
stage3i-validator-count-drift|stage3i-push-this-protocol|src/runtime/binary_object/ordinary_leaf.rs|    if push_this_count != 1 {|    if push_this_count > 1 {
stage3i-validator-index-drift|stage3i-push-this-protocol|src/runtime/binary_object/ordinary_leaf.rs|    if push_this_index != Some(0) {|    if push_this_index != Some(1) {
stage3i-publisher-push-this-pre-match-drop|stage3i-push-this-publication|src/runtime/binary_object_publish.rs|    let instruction = match operation {|    if matches!(operation, OrdinaryLeafOp::PushThis) { return Ok(Instruction::Nop); }\n    let instruction = match operation {
stage3i-wire-fnv-drift|stage3i-runtime-evidence|src/runtime/tests.rs|            0x4ec7_e018_7375_d810,|            0x4ec7_e018_7375_d811,
stage3i-runtime-test-ignored|stage3i-runtime-evidence|src/runtime/tests.rs|#[test]\nfn trusted_quickjs_ordinary_push_this_is_exact_typed_and_realm_correct() {|#[test]\n#[ignore]\nfn trusted_quickjs_ordinary_push_this_is_exact_typed_and_realm_correct() {
stage3i-c-oracle-disabled|stage3i-c-oracle|tests/fixtures/function_bytecode_wire.c|    if (expect_ordinary_push_this_completion(compile_context))|    if (0 && expect_ordinary_push_this_completion(compile_context))
stage3i-c-transcript-verdict-drift|stage3i-c-oracle|tests/fixtures/function_bytecode_wire.quickjs-2026-06-04.txt|ordinary-push-this-oracle=passed|ordinary-push-this-oracle=failed
stage3i-c-manifest-source-hash-drift|stage3i-c-oracle|dev-support/quickjs-c-oracles.tsv|e6d93033db5e00b403ab203e598bc66f77d079329680e970d779d63a388ff0c4|e6d93033db5e00b403ab203e598bc66f77d079329680e970d779d63a388ff0c5
stage3i-status-registry-counts-stale|stage3i-status|docs/status.md|The scalar policy remains 30 opcodes; the stage-3I ordinary policy is 132,\nand their union is 133 (111 blocked, one scalar-only, 103 ordinary-only, and 29\nshared registry rows).|The scalar policy remains 30 opcodes; the stage-3H ordinary policy is 131,\nand their union is 132 (112 blocked, one scalar-only, 102 ordinary-only, and 29\nshared registry rows).
stage3i-status-raw8-reblocked|stage3i-status|docs/status.md|stage 3I adds exactly raw 8 `push_this` with its `None` operand and ordinary-only\naudience. Raw 47 `return_async` and raw 112 `to_propkey` remain blocked.|Raw 8 `push_this`, raw 47 `return_async`, and raw 112 `to_propkey` remain\nblocked.
stage3i-status-blocker-vector-stale|stage3i-status|docs/status.md|`1, 5, 2, 1, 3, 7, 16, 15, 25, 4, 9, 11, 5, 4, 3`.|`1, 6, 2, 1, 3, 7, 16, 15, 25, 4, 9, 11, 5, 4, 3`.
stage3i-status-no-parity-erased|stage3i-status|docs/status.md|Stage 3I changes neither the engine Instruction set nor\nVM implementation and adds no source syntax, public API, Test262 admission, or\nFeature Parity claim.|Stage 3I changes neither the engine Instruction set nor\nVM implementation and establishes Feature Parity.
stage3i-status-current-certification-erased|stage3i-status|docs/status.md|This promoted receipt is source-current for Stage 3I and covers|This promoted receipt merely describes
stage3i-status-raw8-coverage-erased|stage3i-status|docs/status.md|covers the raw-8\n`PushThis` admission and its Rust/C evidence|describes the raw-8\n`PushThis` admission and its Rust/C evidence
stage3i-status-terminal-success-erased|stage3i-status|docs/status.md|The canonical `test262-receipt` run completed successfully|The canonical `test262-receipt` run terminated
stage3i-status-exact-six-artifact-erased|stage3i-status|docs/status.md|unique exact six-file artifact `9452593259`|artifact `9452593259`
STAGE3I_CANARIES
expect_full_rewrite_rejected stage3i-validator-if-false-target-zero-erased \
    stage3i-push-this-protocol src/runtime/binary_object/ordinary_leaf.rs \
    '            FunctionOp::IfFalse(0) | FunctionOp::IfTrue(0) | FunctionOp::Goto(0)' \
    '            FunctionOp::IfTrue(0) | FunctionOp::Goto(0)'
expect_full_rewrite_rejected stage3i-validator-if-true-target-zero-erased \
    stage3i-push-this-protocol src/runtime/binary_object/ordinary_leaf.rs \
    '            FunctionOp::IfFalse(0) | FunctionOp::IfTrue(0) | FunctionOp::Goto(0)' \
    '            FunctionOp::IfFalse(0) | FunctionOp::Goto(0)'
expect_full_rewrite_rejected stage3i-validator-goto-target-zero-erased \
    stage3i-push-this-protocol src/runtime/binary_object/ordinary_leaf.rs \
    '            FunctionOp::IfFalse(0) | FunctionOp::IfTrue(0) | FunctionOp::Goto(0)' \
    '            FunctionOp::IfFalse(0) | FunctionOp::IfTrue(0)'
expect_full_rewrite_rejected stage3i-translate-push-this-alias-intercept \
    stage3i-push-this-translation-route \
    src/runtime/binary_object/function_translate/mod.rs \
    '                PendingOperation::Ready(operation) => operation,' \
    $'                PendingOperation::Ready(operation) => {\n                    use FunctionOp as O;\n                    if matches!(operation, O::PushThis) {\n                        continue;\n                    }\n                    operation\n                },'
expect_full_rewrite_rejected stage3i-ordinary-push-this-alias-pre-match \
    stage3i-push-this-ordinary-route \
    src/runtime/binary_object/ordinary_leaf.rs \
    '    match operation {' \
    $'    use FunctionOp as O;\n    if matches!(operation, O::PushThis) {\n        return Ok(OrdinaryLeafOp::Nop);\n    }\n    match operation {'
expect_full_rewrite_rejected stage3i-status-typed-hidden-wrapper \
    stage3i-status docs/status.md \
    'Stage 3I admits raw 8 only as the exact one-to-one typed chain' \
    $'<div hidden>\nStage 3I admits raw 8 only as the exact one-to-one typed chain' \
    $'Stage 3I changes neither the engine Instruction set nor\nVM implementation and adds no source syntax, public API, Test262 admission, or\nFeature Parity claim.' \
    $'Stage 3I changes neither the engine Instruction set nor\nVM implementation and adds no source syntax, public API, Test262 admission, or\nFeature Parity claim.\n</div>'
expect_full_rewrite_rejected stage3i-status-rust-evidence-comment-wrapper \
    stage3i-status docs/status.md \
    'Stage-3I Rust evidence pins compiler-natural strict and sloppy 47-byte' \
    $'<!--\nStage-3I Rust evidence pins compiler-natural strict and sloppy 47-byte' \
    'protocol does not narrow older ordinary bodies.' \
    $'protocol does not narrow older ordinary bodies.\n-->'
expect_full_rewrite_rejected stage3i-status-c-evidence-hidden-wrapper \
    stage3i-status docs/status.md \
    'Stage 3I additionally compiler-naturally emits strict and sloppy' \
    $'<div style="display:none">\nStage 3I additionally compiler-naturally emits strict and sloppy' \
    '`7e90cdbc0c7570050eb983e7ddfdea32914ff9e753dd986a654ac9c56d7ea355`.' \
    $'`7e90cdbc0c7570050eb983e7ddfdea32914ff9e753dd986a654ac9c56d7ea355`.\n</div>'
expect_full_rewrite_rejected stage3h-to-object-stack-effect-drift \
    stage3h-to-object-verifier src/bytecode.rs \
    '            Self::SetName(_) | Self::ToObject | Self::IteratorCheckObject => (1, 1),' \
    '            Self::SetName(_) | Self::ToObject | Self::IteratorCheckObject => (0, 1),'
expect_full_rewrite_rejected stage3h-translate-to-object-erased-second-pass \
    stage3h-to-object-translation-route \
    src/runtime/binary_object/function_translate/mod.rs \
    '                PendingOperation::Ready(operation) => operation,' \
    $'                PendingOperation::Ready(FunctionOp::ToObject) => continue,\n                PendingOperation::Ready(operation) => operation,'
expect_full_rewrite_rejected stage3h-translate-to-object-alias-erased-second-pass \
    stage3h-to-object-translation-route \
    src/runtime/binary_object/function_translate/mod.rs \
    '                PendingOperation::Ready(operation) => operation,' \
    $'                PendingOperation::Ready(operation) => {\n                    use FunctionOp as O;\n                    if matches!(operation, O::ToObject) {\n                        continue;\n                    }\n                    operation\n                },'
expect_full_rewrite_rejected stage3h-to-object-verifier-max-stack-bypass \
    stage3h-to-object-verifier src/bytecode.rs \
    '        record_maximum_depth(&mut maximum, next_depth, declared_max_stack)?;' \
    $'        if matches!(instruction, Instruction::ToObject) {\n            continue;\n        }\n        record_maximum_depth(&mut maximum, next_depth, declared_max_stack)?;'
expect_full_rewrite_rejected stage3h-to-object-verifier-fallthrough-bypass \
    stage3h-to-object-verifier src/bytecode.rs \
    $'            Instruction::ThrowReadOnly(_)\n            | Instruction::ThrowRedeclaration(_)' \
    $'            Instruction::ToObject\n            | Instruction::ThrowReadOnly(_)\n            | Instruction::ThrowRedeclaration(_)'
expect_full_rewrite_rejected stage3h-vm-to-object-nullish-bypass \
    stage3h-to-object-vm src/vm.rs \
    $'                    Value::Null | Value::Undefined => {\n                        return Err(Error::new(ErrorKind::Type, "cannot convert to object"));\n                    }' \
    $'                    Value::Null | Value::Undefined => {\n                        self.stack.push(Value::Undefined);\n                    }'
expect_full_rewrite_rejected stage3h-vm-to-object-pre-match-diversion \
    stage3h-to-object-vm src/vm.rs \
    $'    ) -> Result<Option<Completion>, Error> {\n        match instruction {\n            Instruction::Arguments(kind) =>' \
    $'    ) -> Result<Option<Completion>, Error> {\n        if matches!(instruction, Instruction::ToObject) {\n            return Ok(None);\n        }\n        match instruction {\n            Instruction::Arguments(kind) =>'
expect_full_rewrite_rejected stage3h-vm-to-object-alias-pre-match-diversion \
    stage3h-to-object-vm src/vm.rs \
    $'    ) -> Result<Option<Completion>, Error> {\n        match instruction {\n            Instruction::Arguments(kind) =>' \
    $'    ) -> Result<Option<Completion>, Error> {\n        use Instruction as I;\n        if matches!(instruction, I::ToObject) {\n            return Ok(None);\n        }\n        match instruction {\n            Instruction::Arguments(kind) =>'
expect_full_rewrite_rejected stage3h-vm-to-object-helper-pre-match-diversion \
    stage3h-to-object-vm src/vm.rs \
    $'    ) -> Result<Option<Completion>, Error> {\n        match instruction {\n            Instruction::Arguments(kind) =>' \
    $'    ) -> Result<Option<Completion>, Error> {\n        fn diverts_to_object(instruction: &Instruction) -> bool {\n            matches!(instruction, Instruction::ToObject)\n        }\n        if diverts_to_object(instruction) {\n            return Ok(None);\n        }\n        match instruction {\n            Instruction::Arguments(kind) =>'
expect_full_rewrite_rejected stage3h-runtime-test-macro-shadow \
    stage3h-runtime-evidence src/runtime/tests.rs \
    $'fn trusted_quickjs_ordinary_to_object_verification_rolls_back_and_retries() {\n    let mut fallthrough = QUICKJS_ORDINARY_TO_OBJECT_BC5.to_vec();' \
    $'fn trusted_quickjs_ordinary_to_object_verification_rolls_back_and_retries() {\n    macro_rules! assert_eq { ($($tokens:tt)*) => {}; }\n    let mut fallthrough = QUICKJS_ORDINARY_TO_OBJECT_BC5.to_vec();'
expect_full_rewrite_rejected stage3h-status-typed-hidden-wrapper \
    stage3h-status docs/status.md \
    'Stage 3H admits raw 111 only as the exact one-to-one typed chain' \
    $'<div hidden>\nStage 3H admits raw 111 only as the exact one-to-one typed chain' \
    $'Stage 3H changes neither production bytecode nor VM\nimplementation and adds no source syntax, public API, Test262 admission, or\nFeature Parity claim.' \
    $'Stage 3H changes neither production bytecode nor VM\nimplementation and adds no source syntax, public API, Test262 admission, or\nFeature Parity claim.\n</div>'
expect_full_rewrite_rejected stage3h-status-inherited-lifecycle-comment-wrapper \
    stage3h-status docs/status.md \
    'retains the Stage-3H raw-111 `ToObject`' \
    $'<!--\nretains the Stage-3H raw-111 `ToObject`' \
    $'raw-177 coverage, and makes no new conformance claim.' \
    $'raw-177 coverage, and makes no new conformance claim.\n-->'
expect_full_rewrite_rejected stage3i-status-lifecycle-comment-wrapper \
    stage3i-status docs/status.md \
    'This promoted receipt is source-current for Stage 3I' \
    $'<!--\nThis promoted receipt is source-current for Stage 3I' \
    $'raw-177 coverage, and makes no new conformance claim.' \
    $'raw-177 coverage, and makes no new conformance claim.\n-->'
expect_full_rewrite_rejected stage3f-translate-nop-erased \
    stage3d-throw-translation-route \
    src/runtime/binary_object/function_translate/mod.rs \
    '                PendingOperation::Ready(operation) => operation,' \
    $'                PendingOperation::Ready(FunctionOp::Nop) => continue,\n                PendingOperation::Ready(operation) => operation,'
expect_full_rewrite_rejected stage3f-status-typed-hidden-div-wrapper \
    stage3f-status docs/status.md \
    'Stage 3F admits raw 177 only as the exact one-to-one typed chain' \
    $'<div hidden>\nStage 3F admits raw 177 only as the exact one-to-one typed chain' \
    'adds no public surface, source syntax, Test262 admission, or Feature Parity claim.' \
    $'adds no public surface, source syntax, Test262 admission, or Feature Parity claim.\n</div>'
expect_full_rewrite_rejected stage3f-status-typed-comment-wrapper \
    stage3f-status docs/status.md \
    'Stage 3F admits raw 177 only as the exact one-to-one typed chain' \
    $'<!--\nStage 3F admits raw 177 only as the exact one-to-one typed chain' \
    'adds no public surface, source syntax, Test262 admission, or Feature Parity claim.' \
    $'adds no public surface, source syntax, Test262 admission, or Feature Parity claim.\n-->'
expect_full_rewrite_rejected stage3i-status-lifecycle-hidden-div-wrapper \
    stage3i-status docs/status.md \
    'This promoted receipt is source-current for Stage 3I' \
    $'<div style="display:none">\nThis promoted receipt is source-current for Stage 3I' \
    $'raw-177 coverage, and makes no new conformance claim.' \
    $'raw-177 coverage, and makes no new conformance claim.\n</div>'
expect_full_rewrite_rejected stage3f-status-source-stale-appended \
    stage3f-status docs/status.md \
    'The same oracle pins compatible 32-bit `scope_next` wrapping' \
    $'Stage 3F is source-stale.\n\nThe same oracle pins compatible 32-bit `scope_next` wrapping'
expect_full_rewrite_rejected stage3f-status-source-ahead-authenticated-appended \
    stage3f-status docs/status.md \
    'The same oracle pins compatible 32-bit `scope_next` wrapping' \
    $'Stage 3F is source-ahead and authenticated.\n\nThe same oracle pins compatible 32-bit `scope_next` wrapping'
expect_full_rewrite_rejected stage3f-nop-verifier-terminal-bypass \
    stage3f-nop-verifier src/bytecode.rs \
    $'            Instruction::ThrowReadOnly(_)\n            | Instruction::ThrowRedeclaration(_)' \
    $'            Instruction::Nop\n            | Instruction::ThrowReadOnly(_)\n            | Instruction::ThrowRedeclaration(_)'
expect_full_rewrite_rejected stage3e-synthetic-count-dropped \
    stage3e-read-only-publication src/runtime/binary_object_publish.rs \
    '                        | OrdinaryLeafOp::ThrowReadOnly(_)' \
    ''
expect_full_rewrite_rejected stage3e-subtype-fallback-admission \
    function-translate-semantic-dispatch \
    src/runtime/binary_object/function_translate/mod.rs \
    $'        (Recipe::ThrowReadOnly, NativeOperands::AtomU8 { value, .. }) => Err(\n            FunctionTranslateError::unadmitted_throw_error_subtype(*value),\n        ),' \
    $'        (Recipe::ThrowReadOnly, NativeOperands::AtomU8 { atom, .. }) => {\n            ready(FunctionOp::ThrowReadOnly(project_atom(*atom)?))\n        }'
expect_full_rewrite_rejected stage3e-read-only-stack-pop \
    stage3e-read-only-verifier src/bytecode.rs \
    $'            | Self::ThrowReadOnly(_)\n' \
    '' \
    '            | Self::Throw => (1, 0),' \
    $'            | Self::Throw\n            | Self::ThrowReadOnly(_) => (1, 0),'
expect_full_rewrite_rejected stage3e-read-only-verifier-fallthrough \
    stage3d-throw-verifier src/bytecode.rs \
    $'        record_maximum_depth(&mut maximum, next_depth, declared_max_stack)?;\n        // QuickJS `compute_stack_size` stops as soon as a reachable PC crosses' \
    $'        record_maximum_depth(&mut maximum, next_depth, declared_max_stack)?;\n        if matches!(instruction, Instruction::ThrowReadOnly(_)) {\n            enqueue_fallthrough(\n                &mut worklist,\n                pc,\n                VerificationState {\n                    depth: next_depth,\n                    regions: next_regions.clone(),\n                    return_addresses: next_return_addresses.clone(),\n                    super_call_bases: next_super_call_bases.clone(),\n                },\n                code.len(),\n            )?;\n        }\n        // QuickJS `compute_stack_size` stops as soon as a reachable PC crosses'
expect_full_rewrite_rejected stage3e-vm-read-only-pop-bypass \
    stage3e-read-only-completion src/vm.rs \
    $'            Instruction::ThrowReadOnly(index) => {\n                return Err(host.read_only_error(*index)?);\n            }' \
    $'            Instruction::ThrowReadOnly(index) => {\n                self.pop()?;\n                return Err(host.read_only_error(*index)?);\n            }'
expect_full_rewrite_rejected stage3e-status-source-stale-appended \
    stage3i-status docs/status.md \
    $'claim.\n\nThe same oracle pins compatible 32-bit `scope_next` wrapping' \
    $'claim.\n\nThis R3fj receipt is source-stale for Stage 3E.\n\nThe same oracle pins compatible 32-bit `scope_next` wrapping'
expect_full_rewrite_rejected stage3e-crate-test-cfg-excluded \
    stage3e-runtime-evidence src/lib.rs \
    '//! A pure-Rust rewrite of `QuickJS` aiming at semantic feature parity with the' \
    $'#![cfg(not(test))]\n//! A pure-Rust rewrite of `QuickJS` aiming at semantic feature parity with the'
expect_full_rewrite_rejected stage3e-lib-test-target-disabled \
    stage3e-test-target Cargo.toml \
    $'[lib]\nname = "quickjs_oxide"\npath = "src/lib.rs"' \
    $'[lib]\nname = "quickjs_oxide"\npath = "src/lib.rs"\ntest = false'
expect_full_rewrite_rejected stage3e-lib-test-target-rerouted \
    stage3e-test-target Cargo.toml \
    'path = "src/lib.rs"' \
    'path = "src/test_sink.rs"'
expect_full_rewrite_rejected stage3e-crate-targeted-assert-eq-shadow \
    stage3e-runtime-evidence src/lib.rs \
    'pub mod atom;' \
    $'macro_rules! assert_eq {\n    (QUICKJS_ORDINARY_READ_ONLY_BC5.len(), 47) => { () };\n    ($($tokens:tt)*) => { ::core::assert_eq!($($tokens)*) };\n}\n\npub mod atom;'
expect_full_rewrite_rejected stage3e-cross-file-macro-use-assert-eq-shadow \
    stage3e-runtime-evidence src/lib.rs \
    'pub mod atom;' \
    $'#[macro_use]\nmod stage3e_shadow;\n\npub mod atom;' \
    '' '' \
    src/stage3e_shadow.rs \
    $'macro_rules! assert_eq {\n    (QUICKJS_ORDINARY_READ_ONLY_BC5.len(), 47) => { () };\n    ($($tokens:tt)*) => { ::core::assert_eq!($($tokens)*) };\n}\n'
expect_full_rewrite_rejected stage3e-outside-src-path-macro-use-assert-eq-shadow \
    stage3e-runtime-evidence src/lib.rs \
    'pub mod atom;' \
    $'#[macro_use]\n#[path = "../tests/stage3e_shadow.rs"]\nmod stage3e_shadow;\n\npub mod atom;' \
    '' '' \
    tests/stage3e_shadow.rs \
    $'macro_rules! assert_eq {\n    (QUICKJS_ORDINARY_READ_ONLY_BC5.len(), 47) => { () };\n    ($($tokens:tt)*) => { ::core::assert_eq!($($tokens)*) };\n}\n'
expect_full_rewrite_rejected stage3e-status-stage-first-stale-appended \
    stage3i-status docs/status.md \
    'The latest full R3fj execution' \
    $'Stage 3E is source-stale and unauthenticated by this receipt.\n\nThe latest full R3fj execution'
expect_full_rewrite_rejected stage3d-vm-throw-to-return \
    stage3d-throw-completion src/vm.rs \
    '                return self.pop().map(|value| Some(Completion::Throw(value)));' \
    '                return self.pop().map(|value| Some(Completion::Return(value)));'
expect_full_rewrite_rejected stage3d-translate-helper-bypass \
    function-translate-semantic-dispatch \
    src/runtime/binary_object/function_translate/mod.rs \
    $'    let ready = |operation| Ok(PendingExpansion::one(PendingOperation::Ready(operation)));\n    match (recipe, operands) {' \
    $'    let ready = |operation| Ok(PendingExpansion::one(PendingOperation::Ready(operation)));\n    if matches!(recipe, Recipe::Throw) {\n        return ready(FunctionOp::Return);\n    }\n    match (recipe, operands) {'
expect_full_rewrite_rejected stage3d-translate-post-lowering-remap \
    stage3d-throw-translation-route \
    src/runtime/binary_object/function_translate/mod.rs \
    '    Ok(FunctionCode::new(output.into_boxed_slice()))' \
    $'    let output = output\n        .into_iter()\n        .map(|instruction| {\n            let diagnostic = instruction.rejection_diagnostic();\n            let operation = match instruction.into_operation() {\n                FunctionOp::Throw => FunctionOp::Return,\n                operation => operation,\n            };\n            FunctionInstruction::new(InstructionAudience::OrdinaryOnly, diagnostic, operation)\n        })\n        .collect::<Vec<_>>();\n    Ok(FunctionCode::new(output.into_boxed_slice()))'
expect_full_rewrite_rejected stage3d-ordinary-helper-bypass \
    ordinary-leaf-translated-code \
    src/runtime/binary_object/ordinary_leaf.rs \
    $'    match operation {\n        FunctionOp::Nop => Ok(OrdinaryLeafOp::Nop),\n        FunctionOp::Object => Ok(OrdinaryLeafOp::Object),\n        FunctionOp::ToObject => Ok(OrdinaryLeafOp::ToObject),\n        FunctionOp::PushThis => Ok(OrdinaryLeafOp::PushThis),\n        FunctionOp::PushI32(value) => Ok(OrdinaryLeafOp::PushI32(*value)),' \
    $'    if matches!(operation, FunctionOp::Throw) {\n        return Ok(OrdinaryLeafOp::Return);\n    }\n    match operation {\n        FunctionOp::Nop => Ok(OrdinaryLeafOp::Nop),\n        FunctionOp::Object => Ok(OrdinaryLeafOp::Object),\n        FunctionOp::ToObject => Ok(OrdinaryLeafOp::ToObject),\n        FunctionOp::PushThis => Ok(OrdinaryLeafOp::PushThis),\n        FunctionOp::PushI32(value) => Ok(OrdinaryLeafOp::PushI32(*value)),'
expect_full_rewrite_rejected stage3d-publisher-helper-bypass \
    ordinary-leaf-consumer-lowering \
    src/runtime/binary_object_publish.rs \
    $'    let instruction = match operation {\n        OrdinaryLeafOp::Nop => Instruction::Nop,\n        OrdinaryLeafOp::Object => Instruction::Object,\n        OrdinaryLeafOp::ToObject => Instruction::ToObject,\n        OrdinaryLeafOp::PushThis => Instruction::PushThis,\n        OrdinaryLeafOp::PushI32(value) => Instruction::PushI32(value),' \
    $'    if matches!(&operation, OrdinaryLeafOp::Throw) {\n        return Ok(Instruction::Return);\n    }\n    let instruction = match operation {\n        OrdinaryLeafOp::Nop => Instruction::Nop,\n        OrdinaryLeafOp::Object => Instruction::Object,\n        OrdinaryLeafOp::ToObject => Instruction::ToObject,\n        OrdinaryLeafOp::PushThis => Instruction::PushThis,\n        OrdinaryLeafOp::PushI32(value) => Instruction::PushI32(value),'
expect_full_rewrite_rejected stage3d-throw-stack-effect-drift \
    stage3d-throw-verifier src/bytecode.rs \
    '            | Self::Throw => (1, 0),' \
    '            | Self::Throw => (1, 1),'
expect_full_rewrite_rejected stage3d-throw-verifier-fallthrough \
    stage3d-throw-verifier src/bytecode.rs \
    $'        record_maximum_depth(&mut maximum, next_depth, declared_max_stack)?;\n        // QuickJS `compute_stack_size` stops as soon as a reachable PC crosses' \
    $'        record_maximum_depth(&mut maximum, next_depth, declared_max_stack)?;\n        if matches!(instruction, Instruction::Throw) {\n            enqueue_fallthrough(\n                &mut worklist,\n                pc,\n                VerificationState {\n                    depth: next_depth,\n                    regions: next_regions.clone(),\n                    return_addresses: next_return_addresses.clone(),\n                    super_call_bases: next_super_call_bases.clone(),\n                },\n                code.len(),\n            )?;\n        }\n        // QuickJS `compute_stack_size` stops as soon as a reachable PC crosses'
expect_full_rewrite_rejected stage3d-execute-throw-bypass \
    stage3d-throw-completion src/vm.rs \
    $'                Ok(InterpreterExit::Complete(Completion::Throw(value))) => value,\n                Ok(InterpreterExit::Suspend(_)) =>' \
    $'                Ok(InterpreterExit::Complete(Completion::Throw(value)))\n                    if matches!(&value, Value::Undefined) => {\n                        return Ok(Completion::Return(value));\n                    }\n                Ok(InterpreterExit::Complete(Completion::Throw(value))) => value,\n                Ok(InterpreterExit::Suspend(_)) =>'
expect_full_rewrite_rejected stage3d-raise-bypass \
    stage3d-throw-completion src/vm.rs \
    $'    ) -> Result<Option<Completion>, Error> {\n        host.ensure_backtrace(&value)?;\n        loop {' \
    $'    ) -> Result<Option<Completion>, Error> {\n        if matches!(value, Value::Undefined) {\n            return Ok(Some(Completion::Throw(value)));\n        }\n        host.ensure_backtrace(&value)?;\n        loop {'
expect_full_rewrite_rejected stage3d-execute-inner-post-route-throw-return \
    stage3d-throw-critical-route src/vm.rs \
    '            if let Some(completion) = self.execute_hot_instruction(code, instruction, host)? {' \
    $'            if matches!(instruction, Instruction::Throw) {\n                return Ok(InterpreterExit::Complete(Completion::Return(Value::Undefined)));\n            }\n            if let Some(completion) = self.execute_hot_instruction(code, instruction, host)? {'
expect_full_rewrite_rejected stage3d-execute-hot-entry-throw-return \
    stage3d-throw-critical-route src/vm.rs \
    $'    ) -> Result<Option<Completion>, Error> {\n        match instruction {\n            Instruction::Nop => {}' \
    $'    ) -> Result<Option<Completion>, Error> {\n        if matches!(instruction, Instruction::Throw) {\n            return self.pop().map(|value| Some(Completion::Return(value)));\n        }\n        match instruction {\n            Instruction::Nop => {}'
expect_full_rewrite_rejected stage3d-execute-published-throw-return \
    stage3d-throw-critical-route src/vm.rs \
    $'        )\n        .execute(code, host)\n    }' \
    $'        )\n        .execute(code, host)\n        .map(|completion| match completion {\n            Completion::Throw(value) => Completion::Return(value),\n            completion => completion,\n        })\n    }'
expect_full_rewrite_rejected stage3d-bytecode-normal-bridge-throw-return \
    stage3d-throw-critical-route src/runtime/vm_host.rs \
    $'        let result = Vm::new().execute_published(input, &mut host);\n        active_frame.finish()?;\n        result.map_err(RuntimeError::Engine)\n    }\n}' \
    $'        let result = Vm::new().execute_published(input, &mut host);\n        active_frame.finish()?;\n        result\n            .map(|completion| match completion {\n                Completion::Throw(value) => Completion::Return(value),\n                completion => completion,\n            })\n            .map_err(RuntimeError::Engine)\n    }\n}'
expect_full_rewrite_rejected stage3d-bytecode-module-bridge-throw-return \
    stage3d-throw-critical-route src/runtime/vm_host.rs \
    '            return result.map_err(RuntimeError::Engine);' \
    $'            return result\n                .map(|completion| match completion {\n                    Completion::Throw(value) => Completion::Return(value),\n                    completion => completion,\n                })\n                .map_err(RuntimeError::Engine);'
expect_full_rewrite_rejected stage3d-call-internal-cleanup-throw-return \
    stage3d-throw-critical-route src/runtime/native_dispatch.rs \
    '        frame_error.map_or(result, Err)' \
    $'        frame_error.map_or(result, Err).map(|completion| match completion {\n            Completion::Throw(value) => Completion::Return(value),\n            completion => completion,\n        })'
expect_full_rewrite_rejected stage3d-vm-host-call-throw-return \
    stage3d-throw-critical-route src/runtime/vm_host.rs \
    $'        self.runtime\n            .call_value_internal(self.current_realm, function, this_value, &arguments)\n            .map_err(runtime_error_to_vm_error)' \
    $'        self.runtime\n            .call_value_internal(self.current_realm, function, this_value, &arguments)\n            .map(|completion| match completion {\n                Completion::Throw(value) => Completion::Return(value),\n                completion => completion,\n            })\n            .map_err(runtime_error_to_vm_error)'
expect_full_rewrite_rejected stage3d-call-value-throw-return \
    stage3d-throw-critical-route src/runtime/internal_methods.rs \
    $'            DirectCallTarget::Callable(callable) => {\n                self.call_internal(caller_realm, &callable, this_value, arguments)\n            }' \
    $'            DirectCallTarget::Callable(callable) => self\n                .call_internal(caller_realm, &callable, this_value, arguments)\n                .map(|completion| match completion {\n                    Completion::Throw(value) => Completion::Return(value),\n                    completion => completion,\n                }),'
expect_full_rewrite_rejected stage3d-context-call-throw-return \
    stage3d-throw-critical-route src/runtime/context.rs \
    $'        let completion = self\n            .runtime\n            .call_internal(self.realm, callable, this_value, arguments)?;\n        self.finish_completion(completion)' \
    $'        let completion = self\n            .runtime\n            .call_internal(self.realm, callable, this_value, arguments)?;\n        let completion = match completion {\n            Completion::Throw(value) => Completion::Return(value),\n            completion => completion,\n        };\n        self.finish_completion(completion)'
expect_full_rewrite_rejected stage3d-backtrace-hook-noop \
    stage3d-throw-critical-route src/runtime/vm_host.rs \
    $'    fn ensure_backtrace(&mut self, value: &Value) -> Result<(), Error> {\n        self.runtime' \
    $'    fn ensure_backtrace(&mut self, value: &Value) -> Result<(), Error> {\n        let _ = value;\n        return Ok(());\n        self.runtime'
expect_full_rewrite_rejected stage3d-iterator-close-hook-noop \
    stage3d-throw-critical-route src/runtime/vm_host.rs \
    $'    ) -> Result<IteratorCloseOutcome, Error> {\n        let return_key = self' \
    $'    ) -> Result<IteratorCloseOutcome, Error> {\n        return Ok(IteratorCloseOutcome::Closed);\n        let return_key = self'
expect_full_rewrite_rejected stage3d-pending-writer-noop \
    stage3d-throw-pending src/runtime.rs \
    $'    fn set_pending_exception(&self, value: Value) -> Result<(), RuntimeError> {\n        let _operation = self.operation();' \
    $'    fn set_pending_exception(&self, value: Value) -> Result<(), RuntimeError> {\n        let _ = value;\n        return Ok(());\n        let _operation = self.operation();'
expect_full_rewrite_rejected stage3d-pending-reader-noop \
    stage3d-throw-pending src/runtime.rs \
    $'    fn take_pending_exception(&self) -> Result<Option<Value>, RuntimeError> {\n        let _operation = self.operation();' \
    $'    fn take_pending_exception(&self) -> Result<Option<Value>, RuntimeError> {\n        return Ok(None);\n        let _operation = self.operation();'
expect_full_rewrite_rejected stage3d-pending-observer-false \
    stage3d-throw-pending src/runtime.rs \
    $'    fn has_pending_exception(&self) -> bool {\n        let _operation = self.operation();\n        self.0.state.borrow().pending_exception.is_some()\n    }' \
    $'    fn has_pending_exception(&self) -> bool {\n        false\n    }'
expect_full_rewrite_rejected stage3d-rust-raw48-wire-alias \
    stage3d-runtime-evidence src/runtime/tests.rs \
    $'    0x01, 0x01, 0x00, 0x00, 0x00, 0x02, 0x01, 0x00, 0x01, 0x00, 0x00, 0xcf, 0x30,\n];' \
    $'    0x01, 0x01, 0x00, 0x00, 0x00, 0x02, 0x01, 0x00, 0x01, 0x00, 0x00, 0xcf, 0x2f,\n];'
expect_full_rewrite_rejected stage3d-nonordinary-metadata-test-ignored \
    stage3d-runtime-evidence src/runtime/tests.rs \
    $'#[test]\nfn trusted_quickjs_ordinary_throw_rejects_nonordinary_metadata_transactionally() {' \
    $'#[test]\n#[ignore = "gate mutation"]\nfn trusted_quickjs_ordinary_throw_rejects_nonordinary_metadata_transactionally() {'
expect_full_rewrite_rejected stage3d-runtime-module-assert-eq-shadow \
    stage3d-runtime-evidence src/runtime/tests.rs \
    'use crate::JsBigInt;' \
    $'use crate::JsBigInt;\n\nmacro_rules! assert_eq { ($($tokens:tt)*) => {}; }'
expect_full_rewrite_rejected stage3d-runtime-module-assert-shadow \
    stage3d-runtime-evidence src/runtime/tests.rs \
    'use crate::JsBigInt;' \
    $'use crate::JsBigInt;\n\nmacro_rules! assert { ($($tokens:tt)*) => {}; }'
expect_full_rewrite_rejected stage3d-runtime-module-raw-assert-eq-shadow \
    stage3d-runtime-evidence src/runtime/tests.rs \
    'use crate::JsBigInt;' \
    $'use crate::JsBigInt;\n\nmacro_rules! r#assert_eq { ($($tokens:tt)*) => {}; }'
expect_full_rewrite_rejected stage3d-runtime-test-nested-cfg \
    stage3d-runtime-evidence src/runtime/tests.rs \
    $'#[test]\nfn trusted_quickjs_ordinary_throw_rejects_nonordinary_metadata_transactionally() {' \
    $'#[cfg(any())]\nmod disabled_raw48_metadata {\n#[test]\nfn trusted_quickjs_ordinary_throw_rejects_nonordinary_metadata_transactionally() {' \
    $'    assert!(!context.has_exception());\n}\n\n#[test]\nfn trusted_quickjs_ordinary_read_only_uses_exact_zero_stack_wire_and_type_error() {' \
    $'    assert!(!context.has_exception());\n}\n}\n\n#[test]\nfn trusted_quickjs_ordinary_read_only_uses_exact_zero_stack_wire_and_type_error() {'
expect_full_rewrite_rejected stage3i-status-current-receipt-erased \
    stage3i-status docs/status.md \
    'This promoted receipt is source-current for Stage 3I and covers' \
    'This promoted receipt merely describes'
expect_full_rewrite_rejected stage3i-status-run-tampered \
    stage3i-status docs/status.md \
    'exact-source GitHub Actions run `32497291807`' \
    'exact-source GitHub Actions run `32497291808`'
expect_full_rewrite_rejected stage3i-status-job-tampered \
    stage3i-status docs/status.md \
    'job `96818622699`, authenticates Stage 3I source' \
    'job `96818622700`, authenticates Stage 3I source'
expect_full_rewrite_rejected stage3i-status-artifact-tampered \
    stage3i-status docs/status.md \
    'unique exact six-file artifact `9452593259`' \
    'unique exact six-file artifact `9452593260`'
expect_full_rewrite_rejected stage3i-status-artifact-digest-tampered \
    stage3i-status docs/status.md \
    '97ff9d79784ed27ec2a323544597b28aa2c5b5227b0028e5d3560bbaa22b1bfb' \
    '07ff9d79784ed27ec2a323544597b28aa2c5b5227b0028e5d3560bbaa22b1bfb'
expect_full_rewrite_rejected stage3i-status-baseline-fingerprint-tampered \
    stage3i-status docs/status.md \
    'd21943622773d2b0b978cd2ace5261d5ec41a9400ab36864768470aae71b1d22' \
    '021943622773d2b0b978cd2ace5261d5ec41a9400ab36864768470aae71b1d22'
expect_full_rewrite_rejected stage3i-status-baseline-digest-tampered \
    stage3i-status docs/status.md \
    '2c8dc920428aef4f10be440d7d18fdf72ec0902af4c7482f5be34ed4f25b1215' \
    '0c8dc920428aef4f10be440d7d18fdf72ec0902af4c7482f5be34ed4f25b1215'
expect_full_rewrite_rejected stage3i-status-baseline-run-tampered \
    stage3i-status docs/status.md \
    'identical to Stage 3H run `32419997996`' \
    'identical to Stage 3H run `32419997997`'
expect_full_rewrite_rejected stage3i-status-baseline-artifact-tampered \
    stage3i-status docs/status.md \
    'artifact `9425844939` (SHA-256' \
    'artifact `9425844940` (SHA-256'
stage3i_current_fingerprint=$(
    sed -n 's/^engine_semantics_sha256=//p' \
        "$repository_root/dev-support/test262/current.conf"
)
stage3i_current_source=$(
    sed -n 's/^engine_semantics_source=//p' \
        "$repository_root/dev-support/test262/current.conf"
)
stage3i_current_focused_tsv=$(
    sed -n 's/^focused_tsv_sha256=//p' \
        "$repository_root/dev-support/test262/current.conf"
)
stage3i_current_focused_jsonl=$(
    sed -n 's/^focused_jsonl_sha256=//p' \
        "$repository_root/dev-support/test262/current.conf"
)
stage3i_current_full_tsv=$(
    sed -n 's/^full_tsv_sha256=//p' \
        "$repository_root/dev-support/test262/current.conf"
)
stage3i_current_full_jsonl=$(
    sed -n 's/^full_jsonl_sha256=//p' \
        "$repository_root/dev-support/test262/current.conf"
)
[[ $stage3i_current_fingerprint =~ ^[0-9a-f]{64}$ ]] \
    || die "Stage3I receipt canary requires one canonical current.conf fingerprint"
[[ $stage3i_current_source =~ ^[0-9a-f]{40}$ ]] \
    || die "Stage3I receipt canary requires one canonical current.conf source"
[[ $stage3i_current_focused_tsv =~ ^[0-9a-f]{64}$ ]] \
    || die "Stage3I receipt canary requires one canonical current.conf focused TSV hash"
[[ $stage3i_current_focused_jsonl =~ ^[0-9a-f]{64}$ ]] \
    || die "Stage3I receipt canary requires one canonical current.conf focused JSONL hash"
[[ $stage3i_current_full_tsv =~ ^[0-9a-f]{64}$ ]] \
    || die "Stage3I receipt canary requires one canonical current.conf full TSV hash"
[[ $stage3i_current_full_jsonl =~ ^[0-9a-f]{64}$ ]] \
    || die "Stage3I receipt canary requires one canonical current.conf full JSONL hash"
tamper_stage3i_receipt_hex() {
    local value=$1
    if [[ ${value:0:1} == 0 ]]; then
        printf '1%s' "${value:1}"
    else
        printf '0%s' "${value:1}"
    fi
}
stage3i_tampered_source=$(tamper_stage3i_receipt_hex "$stage3i_current_source")
stage3i_tampered_fingerprint=$(tamper_stage3i_receipt_hex "$stage3i_current_fingerprint")
stage3i_tampered_focused_tsv=$(tamper_stage3i_receipt_hex "$stage3i_current_focused_tsv")
stage3i_tampered_focused_jsonl=$(tamper_stage3i_receipt_hex "$stage3i_current_focused_jsonl")
stage3i_tampered_full_tsv=$(tamper_stage3i_receipt_hex "$stage3i_current_full_tsv")
stage3i_tampered_full_jsonl=$(tamper_stage3i_receipt_hex "$stage3i_current_full_jsonl")
expect_full_rewrite_rejected stage3i-status-current-source-tampered \
    stage3i-status docs/status.md \
    "$stage3i_current_source" \
    "$stage3i_tampered_source"
expect_full_rewrite_rejected stage3i-status-current-fingerprint-tampered \
    stage3i-status docs/status.md \
    "$stage3i_current_fingerprint" \
    "$stage3i_tampered_fingerprint"
expect_full_rewrite_rejected stage3i-status-current-focused-tsv-tampered \
    stage3i-status docs/status.md \
    "$stage3i_current_focused_tsv" \
    "$stage3i_tampered_focused_tsv"
expect_full_rewrite_rejected stage3i-status-current-focused-jsonl-tampered \
    stage3i-status docs/status.md \
    "$stage3i_current_focused_jsonl" \
    "$stage3i_tampered_focused_jsonl"
expect_full_rewrite_rejected stage3i-status-current-full-tsv-tampered \
    stage3i-status docs/status.md \
    "$stage3i_current_full_tsv" \
    "$stage3i_tampered_full_tsv"
expect_full_rewrite_rejected stage3i-status-current-full-jsonl-tampered \
    stage3i-status docs/status.md \
    "$stage3i_current_full_jsonl" \
    "$stage3i_tampered_full_jsonl"
expect_full_rewrite_rejected stage3i-status-receipt-boundary-erased \
    stage3i-status docs/status.md \
    $'claim.\n\nThe same oracle pins compatible 32-bit `scope_next` wrapping' \
    $'claim.\nThe same oracle pins compatible 32-bit `scope_next` wrapping'
expect_full_rewrite_rejected stage3i-status-stale-contradiction-appended \
    stage3i-status docs/status.md \
    $'claim.\n\nThe same oracle pins compatible 32-bit `scope_next` wrapping' \
    $'claim.\n\nThis R3fj receipt is source-stale for Stage 3I.\n\nThe same oracle pins compatible 32-bit `scope_next` wrapping'
expect_full_rewrite_rejected stage3i-status-unauthenticated-contradiction-appended \
    stage3i-status docs/status.md \
    $'claim.\n\nThe same oracle pins compatible 32-bit `scope_next` wrapping' \
    $'claim.\n\nStage 3I has yet to be authenticated.\n\nThe same oracle pins compatible 32-bit `scope_next` wrapping'
expect_full_rewrite_rejected stage3i-status-stage3h-only-contradiction-appended \
    stage3i-status docs/status.md \
    $'claim.\n\nThe same oracle pins compatible 32-bit `scope_next` wrapping' \
    $'claim.\n\nOnly Stage 3H is authenticated by this receipt.\n\nThe same oracle pins compatible 32-bit `scope_next` wrapping'
expect_full_rewrite_rejected stage3i-status-not-authenticated-contradiction-appended \
    stage3i-status docs/status.md \
    $'claim.\n\nThe same oracle pins compatible 32-bit `scope_next` wrapping' \
    $'claim.\n\nStage 3I is not authenticated.\n\nThe same oracle pins compatible 32-bit `scope_next` wrapping'
expect_full_rewrite_rejected stage3i-status-receipt-stage3h-only-contradiction-appended \
    stage3i-status docs/status.md \
    $'claim.\n\nThe same oracle pins compatible 32-bit `scope_next` wrapping' \
    $'claim.\n\nThis receipt only authenticates Stage 3H.\n\nThe same oracle pins compatible 32-bit `scope_next` wrapping'
expect_full_rewrite_rejected stage3i-status-stage3g-only-contradiction-appended \
    stage3i-status docs/status.md \
    $'claim.\n\nThe same oracle pins compatible 32-bit `scope_next` wrapping' \
    $'claim.\n\nOnly Stage 3G is authenticated by this receipt.\n\nThe same oracle pins compatible 32-bit `scope_next` wrapping'
expect_full_rewrite_rejected stage3i-status-stage3f-only-contradiction-appended \
    stage3i-status docs/status.md \
    $'claim.\n\nThe same oracle pins compatible 32-bit `scope_next` wrapping' \
    $'claim.\n\nOnly Stage 3F is authenticated by this receipt.\n\nThe same oracle pins compatible 32-bit `scope_next` wrapping'
expect_full_rewrite_rejected stage3i-status-pending-promotion-contradiction-appended \
    stage3i-status docs/status.md \
    $'claim.\n\nThe same oracle pins compatible 32-bit `scope_next` wrapping' \
    $'claim.\n\nStage 3I is pending a separate exact-source receipt promotion.\n\nThe same oracle pins compatible 32-bit `scope_next` wrapping'
expect_full_rewrite_rejected stage3i-status-html-comment-wrapper \
    stage3i-status docs/status.md \
    'The latest full R3fj execution' \
    $'<!--\n\nThe latest full R3fj execution' \
    'raw-177 coverage, and makes no new conformance claim.' \
    $'raw-177 coverage, and makes no new conformance claim.\n\n-->'
expect_full_rewrite_rejected stage3i-status-fenced-code-wrapper \
    stage3i-status docs/status.md \
    'The latest full R3fj execution' \
    $'```text\n\nThe latest full R3fj execution' \
    'raw-177 coverage, and makes no new conformance claim.' \
    $'raw-177 coverage, and makes no new conformance claim.\n\n```'
expect_full_rewrite_rejected stage3i-status-hidden-html-wrapper \
    stage3i-status docs/status.md \
    'The latest full R3fj execution' \
    $'<div hidden>\n\nThe latest full R3fj execution' \
    'raw-177 coverage, and makes no new conformance claim.' \
    $'raw-177 coverage, and makes no new conformance claim.\n\n</div>'
stage3i_status_receipt_paragraph=$(
    awk '
        found && /^[[:space:]]*$/ { exit }
        /The latest full R3fj execution/ { found = 1 }
        found { print }
    ' "$repository_root/docs/status.md"
)
[[ $stage3i_status_receipt_paragraph == *'This promoted receipt is source-current for Stage 3I and covers'* ]] \
    || die "Stage3I indented-code canary could not locate the promoted receipt"
stage3i_status_indented_receipt=$(printf '%s\n' \
    "$stage3i_status_receipt_paragraph" | sed 's/^/    /')
expect_full_rewrite_rejected stage3i-status-indented-code-wrapper \
    stage3i-status docs/status.md \
    "$stage3i_status_receipt_paragraph" \
    "$stage3i_status_indented_receipt"
expect_full_rewrite_rejected stage3i-status-focused-lines-drift \
    stage3i-status dev-support/test262/current.conf \
    'focused_tsv_lines=6857' \
    'focused_tsv_lines=6858'
expect_full_rewrite_rejected stage3i-status-focused-summary-drift \
    stage3i-status dev-support/test262/current.conf \
    'focused_summary=pass=6844' \
    'focused_summary=pass=6843 fail-runtime=1'
run_stage3i_receipt_escape_canaries \
    "$tmp_dir/stage3i-receipt-escape-canaries"
expect_full_rewrite_rejected ordinary-typeof-undefined-html-dda-collapse \
    ordinary-leaf-engine-semantics src/vm.rs \
    '                let is_undefined = matches!(value, Value::Undefined) || host.is_html_dda(&value)?;' \
    '                let is_undefined = matches!(value, Value::Undefined);'
expect_full_rewrite_rejected translate-resolve-target-offset \
    function-translate-control-flow src/runtime/binary_object/function_translate/mod.rs \
    $'        .copied()\n        .ok_or_else(FunctionTranslateError::invalid_branch_target)' \
    $'        .copied()\n        .map(|target| target.saturating_add(1))\n        .ok_or_else(FunctionTranslateError::invalid_branch_target)'
expect_rewrite_rejected scalar-unary-chain-fold scalar-script-translated-code \
    src/runtime/binary_object/scalar_script.rs \
    '        unary_ops.push(ScalarUnaryOp::from_translated(*operation));' \
    '        if unary_ops.last() != Some(&ScalarUnaryOp::from_translated(*operation)) { unary_ops.push(ScalarUnaryOp::from_translated(*operation)); }'
expect_rewrite_rejected scalar-unary-chain-reorder scalar-script-translated-code \
    src/runtime/binary_object/scalar_script.rs \
    '        unary_ops.push(ScalarUnaryOp::from_translated(*operation));' \
    '        unary_ops.insert(0, ScalarUnaryOp::from_translated(*operation));'
expect_rewrite_rejected scalar-completion-slot-drift scalar-script-translated-code \
    src/runtime/binary_object/scalar_script.rs \
    'FunctionOp::SetLocal(0)' \
    'FunctionOp::SetLocal(1)'
expect_rewrite_rejected scalar-return-kind-drift scalar-script-translated-code \
    src/runtime/binary_object/scalar_script.rs \
    'FunctionOp::Return)' \
    'FunctionOp::OutsideTarget)'
expect_rewrite_rejected scalar-post-projection-early-rejection scalar-script-translated-admission \
    src/runtime/binary_object/scalar_script.rs \
    '    Ok((value, unary_ops))' \
    $'    if false { return unadmitted("early rejection"); }\n    Ok((value, unary_ops))'
expect_rewrite_rejected scalar-bigint-infallible-copy scalar-script-bigint-copy \
    src/runtime/binary_object/scalar_script.rs \
    'copy.try_reserve_exact(bytes.len())' \
    'copy.reserve(bytes.len())'
expect_rewrite_rejected scalar-string-utf8-misdecode scalar-script-string-copy \
    src/runtime/binary_object/scalar_script.rs \
    'copy_utf16(bytes.iter().copied().map(u16::from), bytes.len())' \
    'copy_utf16(String::from_utf8_lossy(bytes).encode_utf16(), bytes.len())'
expect_rewrite_rejected scalar-constant-pairing-bypass scalar-script-translated-admission \
    src/runtime/binary_object/scalar_script.rs \
    '    let value = match (push, function.constants()) {' \
    $'    let value = ScalarValueDraft::Float64Bits(0);\n    let _reviewed_pair = match (push, function.constants()) {'
expect_rewrite_rejected scalar-input-atom-slot-widening scalar-script-translated-admission \
    src/runtime/binary_object/scalar_script.rs \
    'image.input_atom_slot_count() != 0' \
    'image.input_atom_slot_count() != 2'
expect_rewrite_rejected scalar-admission-early-success scalar-script-translated-admission \
    src/runtime/binary_object/scalar_script.rs \
    '    let translated = translate_function(image, root, TranslationTarget::Scalar)' \
    '    return Ok((ScalarValueDraft::EmptyString, Box::default())); let translated = translate_function(image, root, TranslationTarget::Scalar)'
expect_rewrite_rejected scalar-label-error-bypass scalar-script-translated-admission \
    src/runtime/binary_object/scalar_script.rs \
    '    if error.is_label_target_error() {' \
    '    if false && error.is_label_target_error() {'
expect_rewrite_rejected scalar-input-origin-zero-bypass scalar-native-atom-consumer \
    src/runtime/binary_object/scalar_script.rs \
    '        0 if atom.originates_from_input_atom_table() => {' \
    '        0 if false && atom.originates_from_input_atom_table() => {'
expect_rewrite_rejected scalar-input-origin-one-bypass scalar-native-atom-consumer \
    src/runtime/binary_object/scalar_script.rs \
    '        1 if !atom.originates_from_input_atom_table() => {' \
    '        1 if false && !atom.originates_from_input_atom_table() => {'
expect_rewrite_rejected scalar-private-identity-admission scalar-native-atom-consumer \
    src/runtime/binary_object/scalar_script.rs \
    '        AtomOperandClass::Private => unadmitted("private atom is not a String value"),' \
    '        AtomOperandClass::Private => project_atom_string_spelling(atom),'
expect_rewrite_rejected scalar-symbol-identity-admission scalar-native-atom-consumer \
    src/runtime/binary_object/scalar_script.rs \
    '        AtomOperandClass::Symbol => unadmitted("symbol atom is not a String value"),' \
    '        AtomOperandClass::Symbol => project_atom_string_spelling(atom),'
expect_rewrite_rejected scalar-index-identity-collapse scalar-native-atom-consumer \
    src/runtime/binary_object/scalar_script.rs \
    '            .map(ScalarValueDraft::IntegerAtomString)' \
    '            .map(|_| ScalarValueDraft::IntegerAtomString(0))'
expect_rewrite_rejected scalar-error-missing-unadmitted scalar-script-error-shape \
    src/runtime/binary_object/scalar_script.rs \
    '    Unadmitted(String),' \
    '    Rejected(String),'
expect_rejected scalar-extra-visible-item scalar-script-visible-item-set \
    src/runtime/binary_object/scalar_script.rs \
    'pub(in crate::runtime) fn leak_image() {}'
expect_rewrite_rejected scalar-unary-visibility-widening scalar-unary-operation-shape \
    src/runtime/binary_object/scalar_script.rs \
    'pub(in crate::runtime) enum ScalarUnaryOp {' \
    'pub(crate) enum ScalarUnaryOp {'
expect_rejected scalar-helper-escape scalar-script-helper-set \
    src/runtime/binary_object/scalar_script.rs \
    'fn admit_unary_without_sidecars() {}'
expect_rewrite_rejected consumer-publication-visibility-widening binary-object-consumer-publication \
    src/runtime/binary_object_publish.rs \
    '    pub(super) fn read_trusted_scalar_script_in_realm(' \
    '    pub(crate) fn read_trusted_scalar_script_in_realm('
expect_full_rewrite_rejected consumer-source-include binary-object-consumer-source-include \
    src/runtime/binary_object_publish.rs \
    $'#[cfg(test)]\nmod tests {' \
    $'include!("scalar_unary_escape.rs");\n\n#[cfg(test)]\nmod tests {'
expect_full_rewrite_rejected consumer-private-module binary-object-consumer-top-level-item-set \
    src/runtime/binary_object_publish.rs \
    $'#[cfg(test)]\nmod tests {' \
    $'mod scalar_unary_escape;\n\n#[cfg(test)]\nmod tests {'
expect_full_rewrite_rejected consumer-private-trait binary-object-consumer-top-level-item-set \
    src/runtime/binary_object_publish.rs \
    $'#[cfg(test)]\nmod tests {' \
    $'trait ScalarUnaryEscape {}\n\n#[cfg(test)]\nmod tests {'
expect_full_rewrite_rejected consumer-helper-escape binary-object-consumer-helper-set \
    src/runtime/binary_object_publish.rs \
    $'#[cfg(test)]\nmod tests {' \
    $'fn publish_unary_without_verification() {}\n\n#[cfg(test)]\nmod tests {'
expect_full_rewrite_rejected consumer-macro-escape binary-object-consumer-macro-set \
    src/runtime/binary_object_publish.rs \
    $'#[cfg(test)]\nmod tests {' \
    $'scalar_unary_escape!();\n\n#[cfg(test)]\nmod tests {'
expect_rewrite_rejected scalar-strict-reader scalar-script-reader-mode \
    src/runtime/binary_object/scalar_script.rs \
    'ReaderMode::QuickJsCompatible' \
    'ReaderMode::Strict'
expect_rewrite_rejected image-atom-visibility-widening image-atom-visibility \
    src/runtime/binary_object/bytecode_image/atoms.rs \
    'pub(super) enum ImageAtom {' \
    'pub(in crate::runtime::binary_object) enum ImageAtom {'
expect_rejected image-atom-reexport image-atom-reexport \
    src/runtime/binary_object/bytecode_image/mod.rs \
    'pub(in crate::runtime::binary_object) use atoms::ImageAtom;'
expect_rejected image-atom-raw-accessor image-atom-escape \
    src/runtime/binary_object/bytecode_image/model.rs \
    'impl ImageLocalVariable { pub(in crate::runtime::binary_object) const fn raw_name(&self) -> ImageAtom { self.name } }'
expect_rejected bytecode-image-trait-raw-u32 bytecode-image-implementation-set \
    src/runtime/binary_object/bytecode_image/model.rs \
    'impl From<&BytecodeImage> for Vec<u32> { fn from(image: &BytecodeImage) -> Self { image.functions().iter().flat_map(|function| function.envelope().code().atom_relocations()).filter_map(|relocation| match relocation.atom() { ImageAtom::Null => None, ImageAtom::Index(value) => Some(value), ImageAtom::Predefined(atom) => Some(atom.raw()), ImageAtom::Dynamic(atom) => Some(atom.zero_based()), }).collect() } }'
expect_rejected bytecode-image-type-alias bytecode-image-alias \
    src/runtime/binary_object/bytecode_image/model.rs \
    'type BytecodeImageAlias = BytecodeImage;'
expect_rewrite_rejected bytecode-image-raw-u32-method bytecode-image-visible-method-set \
    src/runtime/binary_object/bytecode_image/model.rs \
    $'impl BytecodeImage {\n    fn sab_archive_occurrences(&self) {}' \
    $'impl BytecodeImage {\n    pub(in crate::runtime) fn leaked_raw_atom(&self, atom: ImageAtom) -> Option<u32> { match atom { ImageAtom::Null | ImageAtom::Dynamic(_) => None, ImageAtom::Index(raw) => Some(raw), ImageAtom::Predefined(atom) => Some(atom.raw()), } }\n    fn sab_archive_occurrences(&self) {}'
expect_full_rewrite_rejected bytecode-image-helper-indirection bytecode-image-model-seal \
    src/runtime/binary_object/bytecode_image/model.rs \
    $'    pub(in crate::runtime) const fn operand_offset(self) -> u32 {\n        self.operand_offset\n    }' \
    $'    pub(in crate::runtime) const fn operand_offset(self) -> u32 {\n        Self::leak_atom_identity(self.atom)\n    }\n\n    const fn leak_atom_identity(atom: ImageAtom) -> u32 {\n        match atom {\n            ImageAtom::Null => 0,\n            ImageAtom::Index(value) => value,\n            ImageAtom::Predefined(atom) => atom.raw(),\n            ImageAtom::Dynamic(atom) => atom.zero_based(),\n        }\n    }'
expect_rewrite_rejected pinned-eval-identity-drift scalar-script-atom-predicate \
    src/runtime/binary_object/bytecode_image/model.rs \
    'const PINNED_EVAL_ATOM_RAW: u32 = 84;' \
    'const PINNED_EVAL_ATOM_RAW: u32 = 85;'
expect_rejected native-plan-tuple-raw-atom native-plan-type-set \
    src/runtime/binary_object/bytecode_image/native_plan.rs \
    'struct HiddenRawAtom(ImageAtom);'
expect_rejected native-plan-private-raw-helper native-plan-function-set \
    src/runtime/binary_object/bytecode_image/native_plan.rs \
    'fn leaked_raw_atom(atom: ImageAtom) -> ImageAtom { atom }'
expect_rejected native-plan-unicode-raw-helper native-plan-function-set \
    src/runtime/binary_object/bytecode_image/native_plan.rs \
    'fn 泄漏(atom: ImageAtom) -> ImageAtom { atom }'
expect_rejected native-plan-unicode-type-alias native-plan-expansion \
    src/runtime/binary_object/bytecode_image/native_plan.rs \
    'type 泄漏 = ImageAtom;'
expect_rejected native-plan-unicode-module native-plan-expansion \
    src/runtime/binary_object/bytecode_image/native_plan.rs \
    'mod 泄漏 {}'
expect_rejected native-plan-const-raw-helper native-plan-data-item-set \
    src/runtime/binary_object/bytecode_image/native_plan.rs \
    'const RAW_CODE: for<'"'"'a> fn(&'"'"'a ImageCode) -> &'"'"'a [u8] = |code| code.as_bytes();'
expect_rewrite_rejected native-plan-raw-byte-storage native-plan-facade-representation \
    src/runtime/binary_object/bytecode_image/native_plan.rs \
    $'struct DecodedCodePlan<\'image> {\n    instructions: Box<[NativeInstruction<\'image>]>,' \
    $'struct DecodedCodePlan<\'image> {\n    raw_bytes: &\'image [u8],\n    instructions: Box<[NativeInstruction<\'image>]>,'
expect_rewrite_rejected native-plan-runtime-string-storage native-plan-facade-representation \
    src/runtime/binary_object/bytecode_image/native_plan.rs \
    $'struct DecodedCodePlan<\'image> {\n    instructions: Box<[NativeInstruction<\'image>]>,' \
    $'struct DecodedCodePlan<\'image> {\n    runtime_string: JsString,\n    instructions: Box<[NativeInstruction<\'image>]>,'
expect_rewrite_rejected native-plan-module-escape native-plan-expansion \
    src/runtime/binary_object/bytecode_image/native_plan.rs \
    'use std::fmt;' \
    $'use std::fmt;\nmod escape {}'
expect_rewrite_rejected native-plan-include-escape native-plan-expansion \
    src/runtime/binary_object/bytecode_image/native_plan.rs \
    'use std::fmt;' \
    $'use std::fmt;\ninclude!("native_plan_escape.rs");'
expect_rewrite_rejected native-plan-trait-escape native-plan-expansion \
    src/runtime/binary_object/bytecode_image/native_plan.rs \
    'use std::fmt;' \
    $'use std::fmt;\ntrait NativePlanEscape {}'
expect_rejected native-plan-sibling-consumer native-plan-consumer-set \
    src/runtime/binary_object/scalar_script.rs \
    'use super::bytecode_image::native_plan::NativeCodePlan;'
expect_rejected native-plan-second-consumer native-plan-consumer-set \
    src/runtime/binary_object/atoms.rs \
    'fn consume_native_plan(_: NativeCodePlan) {}'
expect_rejected native-plan-facade native-plan-private-stage \
    src/runtime/binary_object/bytecode_image/mod.rs \
    'pub(in crate::runtime::binary_object) use native_plan::NativeCodePlan;'
expect_rewrite_rejected native-plan-atom-class-collapse native-plan-semantic-seal \
    src/runtime/binary_object/bytecode_image/native_plan.rs \
    $'                    PinnedAtomKind::String => NativeAtomClass::String,\n                    PinnedAtomKind::Private => NativeAtomClass::Private,\n                    PinnedAtomKind::Symbol => NativeAtomClass::Symbol,' \
    $'                    PinnedAtomKind::String => NativeAtomClass::String,\n                    PinnedAtomKind::Private => NativeAtomClass::String,\n                    PinnedAtomKind::Symbol => NativeAtomClass::String,'
expect_rewrite_rejected native-plan-raw-pinned-helper native-plan-semantic-seal \
    src/runtime/binary_object/bytecode_image/native_plan.rs \
    '                spelling: atom.spelling(),' \
    '                spelling: if atom.raw() == 0 { atom.spelling() } else { atom.spelling() },'
expect_rewrite_rejected native-plan-dynamic-index-helper native-plan-semantic-seal \
    src/runtime/binary_object/bytecode_image/native_plan.rs \
    'dynamic_atoms.get(index.as_usize())' \
    'dynamic_atoms.get(index.zero_based() as usize)'
expect_rewrite_rejected native-plan-origin-accessor-drift native-plan-semantic-seal \
    src/runtime/binary_object/bytecode_image/native_plan.rs \
    $'    pub(in crate::runtime::binary_object) const fn originates_from_input_atom_table(self) -> bool {\n        self.from_input_atom_table\n    }' \
    $'    pub(in crate::runtime::binary_object) const fn originates_from_input_atom_table(self) -> bool {\n        false\n    }'
expect_rewrite_rejected native-plan-origin-range-widening native-plan-semantic-seal \
    src/runtime/binary_object/bytecode_image/native_plan.rs \
    '.is_some_and(|slot| slot < input_atom_slot_count);' \
    '.is_some_and(|slot| slot <= input_atom_slot_count);'
expect_rewrite_rejected native-plan-label-error-broadening native-plan-semantic-seal \
    src/runtime/binary_object/bytecode_image/native_plan.rs \
    'Self::LabelTargetOutOfRange { .. } | Self::LabelTargetNotInstructionBoundary { .. }' \
    'Self::LabelTargetOutOfRange { .. } | Self::LabelTargetNotInstructionBoundary { .. } | Self::InvalidOpcode { .. }'
expect_rewrite_rejected native-plan-label-error-collapse native-plan-semantic-seal \
    src/runtime/binary_object/bytecode_image/native_plan.rs \
    'Self::LabelTargetOutOfRange { .. } | Self::LabelTargetNotInstructionBoundary { .. }' \
    'Self::LabelTargetOutOfRange { .. }'
expect_rewrite_rejected native-plan-label-accessor-drift native-plan-semantic-seal \
    src/runtime/binary_object/bytecode_image/native_plan.rs \
    $'    pub(in crate::runtime::binary_object) const fn target_pc(self) -> u32 {\n        self.target_pc\n    }' \
    $'    pub(in crate::runtime::binary_object) const fn target_pc(self) -> u32 {\n        self.operand_pc\n    }'
expect_rewrite_rejected native-plan-label-base-drift native-plan-semantic-seal \
    src/runtime/binary_object/bytecode_image/native_plan.rs \
    '.checked_add(u32::from(operand_offset))' \
    '.checked_add(1)'
expect_rewrite_rejected native-plan-format-size-drift native-plan-semantic-seal \
    src/runtime/binary_object/bytecode_image/native_plan.rs \
    '        OpcodeFormat::AtomU8 => 6,' \
    '        OpcodeFormat::AtomU8 => 7,'
expect_rewrite_rejected native-plan-relocation-base-drift native-plan-semantic-seal \
    src/runtime/binary_object/bytecode_image/native_plan.rs \
    $'            let expected = byte_pc\n                .checked_add(1)' \
    $'            let expected = byte_pc\n                .checked_add(0)'
expect_rewrite_rejected native-plan-atom-label-base-drift native-plan-semantic-seal \
    src/runtime/binary_object/bytecode_image/native_plan.rs \
    $'            label: label32(5)?,\n            value: read_u8(instruction, 9, byte_pc, opcode)?,' \
    $'            label: label32(1)?,\n            value: read_u8(instruction, 9, byte_pc, opcode)?,'
expect_rewrite_rejected native-plan-implicit-minus-one-drift native-plan-implicit-opcode-set \
    src/runtime/binary_object/bytecode_image/native_plan.rs \
    'opcode.name() == "push_minus1"' \
    'opcode.name() == "push_0"'
expect_rewrite_rejected native-plan-implicit-local-drift native-plan-implicit-opcode-set \
    src/runtime/binary_object/bytecode_image/native_plan.rs \
    '&["get_loc", "put_loc", "set_loc"]' \
    '&["get_arg", "put_loc", "set_loc"]'
expect_rejected image-public-module image-module-visibility \
    src/runtime/binary_object/bytecode_image/mod.rs \
    'pub(in crate::runtime) mod leaked;'
expect_rejected common-cursor-u64 forbidden-common-cursor-capability \
    src/runtime/binary_object/read_cursor.rs \
    'fn read_u64_le(&mut self) -> u64 { 0 }'
expect_rejected common-cursor-sab-hook forbidden-common-cursor-capability \
    src/runtime/binary_object/read_cursor.rs \
    'fn allows_shared_array_buffers(&self) -> bool { true }'
expect_rejected common-cursor-extra-impl common-cursor-implementation-set \
    src/runtime/binary_object/read_cursor.rs \
    "impl<'input> CheckedReadCursor<'input> for ThirdCursor<'input> {}"
expect_rejected common-cursor-nongeneric-impl common-cursor-implementation-set \
    src/runtime/binary_object/read_cursor.rs \
    "impl CheckedReadCursor<'static> for ThirdCursor {}"
expect_rejected common-cursor-qualified-impl common-cursor-implementation-set \
    src/runtime/binary_object/read_cursor.rs \
    "impl<'input> CheckedReadCursor<'input> for crate::ThirdCursor<'input> {}"
expect_rejected common-cursor-turbofish-impl common-cursor-implementation-set \
    src/runtime/binary_object/read_cursor.rs \
    "impl CheckedReadCursor::<'static> for ThirdCursor {}"
expect_rejected common-cursor-aliased-impl common-cursor-trait-alias \
    src/runtime/binary_object/read_cursor.rs \
    "use self::CheckedReadCursor as Alias; impl Alias<'static> for ThirdCursor {}"
expect_rejected common-cursor-cross-file-alias common-cursor-trait-alias \
    src/runtime/binary_object/atoms.rs \
    "use super::read_cursor::CheckedReadCursor as Alias; impl Alias<'static> for ThirdCursor {}"
expect_rejected common-cursor-extra-seal common-cursor-seal-implementation-set \
    src/runtime/binary_object/read_cursor.rs \
    "impl sealed::Sealed for ThirdCursor {}"
expect_rejected common-cursor-turbofish-seal common-cursor-seal-implementation-set \
    src/runtime/binary_object/read_cursor.rs \
    "impl sealed::Sealed::<> for ThirdCursor {}"
expect_rejected common-cursor-aliased-seal common-cursor-seal-alias \
    src/runtime/binary_object/read_cursor.rs \
    "use self::sealed::Sealed as Seal; impl Seal for ThirdCursor {}"
expect_rejected retired-sab-permit retired-sab-permit \
    src/runtime/binary_object/atoms.rs \
    'struct GraphSabDecodePermit;'
expect_rejected sab-native-token-production-api sab-native-token-implementation-set \
    src/runtime/binary_object/graph/sab_transport.rs \
    'impl NativeSabToken { pub(in crate::runtime) const fn bits(&self) -> u64 { self.native_token_bits } }'
expect_rewrite_rejected sab-native-token-public-field sab-native-token-shape \
    src/runtime/binary_object/graph/sab_transport.rs \
    '    native_token_bits: u64,' \
    '    pub(in crate::runtime) native_token_bits: u64,'
expect_rewrite_rejected sab-native-token-derive sab-native-token-derive \
    src/runtime/binary_object/graph/sab_transport.rs \
    'pub(in crate::runtime) struct NativeSabToken {' \
    '#[derive(Default)] pub(in crate::runtime) struct NativeSabToken {'
expect_rejected sab-extra-cursor-build sab-archive-call-site-set \
    src/runtime/binary_object/graph/sab_transport.rs \
    'fn leak_build(input: SabTransportInput) { let _ = input.build_cursor(); }'
expect_rewrite_rejected sab-test-cursor-build-wrapper sab-input-implementation-set \
    src/runtime/binary_object/graph/sab_transport.rs \
    '        self.build_cursor(mode, wire_limits, graph_limits)' \
    '        audit_build_cursor(self, mode, wire_limits, graph_limits)'
expect_rejected sab-cursor-build-function-item sab-archive-call-site-set \
    src/runtime/binary_object/graph/sab_transport.rs \
    'fn leak_build_item() { let _ = SabTransportInput::build_cursor; }'
expect_rejected sab-image-finalizer-function-item sab-archive-call-site-set \
    src/runtime/binary_object/graph/sab_transport.rs \
    'fn leak_finish_item() { let _ = SabTransportCursor::finish_bytecode_image; }'
expect_rejected sab-graph-finalizer-function-item sab-archive-call-site-set \
    src/runtime/binary_object/graph/sab_transport.rs \
    'fn leak_graph_finish_item() { let _ = SabTransportCursor::finish_graph_archive; }'
expect_rejected sab-cursor-type-alias sab-cursor-alias \
    src/runtime/binary_object/graph/sab_transport.rs \
    "type r#CursorAlias<'a> = SabTransportCursor<'a>;"
expect_rejected sab-cursor-extra-impl sab-cursor-implementation-set \
    src/runtime/binary_object/graph/sab_transport.rs \
    "impl SabTransportCursor<'_> { fn leak(self) {} }"
expect_rejected sab-cursor-extra-literal sab-transport-field-escape \
    src/runtime/binary_object/graph/sab_transport.rs \
    'fn forge_cursor<'"'"'a>(wire: WireCursor<'"'"'a>, occurrences: &'"'"'a [NativeSabToken], archive: SabArchiveState) -> SabTransportCursor<'"'"'a> { SabTransportCursor { cursor_wire: wire, cursor_writer_occurrences: occurrences, cursor_next_occurrence: 0, cursor_archive: archive } }'
expect_rejected sab-cursor-leak-wire sab-transport-field-escape \
    src/runtime/binary_object/graph/sab_transport.rs \
    "impl<'a> SabTransportCursor<'a> { pub(in crate::runtime::binary_object) fn leak_wire(self) -> WireCursor<'a> { self.cursor_wire } }"
expect_rejected sab-input-split sab-transport-field-escape \
    src/runtime/binary_object/graph/sab_transport.rs \
    "impl<'a> SabTransportInput<'a> { pub(in crate::runtime::binary_object) const fn split(&self) -> (&'a [u8], &'a [NativeSabToken]) { (self.transport_wire_bytes, self.transport_writer_occurrences) } }"
expect_rejected sab-graph-owner-detach sab-graph-field-escape \
    src/runtime/binary_object/graph/sab_transport.rs \
    'fn detach_graph(value: ArchivedWireGraph) { let _ = (value.archived_graph_payload, value.archived_graph_shared_backings); }'
expect_rejected sab-graph-extra-impl sab-graph-aggregate-escape \
    src/runtime/binary_object/graph/sab_transport.rs \
    'impl ArchivedWireGraph { fn leak_graph(&self) -> &WireGraph { &self.archived_graph_payload } }'
expect_rewrite_rejected sab-public-graph-field sab-graph-aggregate-shape \
    src/runtime/binary_object/graph/sab_transport.rs \
    '    archived_graph_payload: WireGraph,' \
    '    pub(in crate::runtime::binary_object) archived_graph_payload: WireGraph,'
expect_rejected sab-image-owner-detach sab-image-field-escape \
    src/runtime/binary_object/graph/sab_transport.rs \
    'fn detach(value: ArchivedBytecodeImage) { let _ = value.archived_image_payload; }'
expect_rejected sab-image-external-detach sab-image-field-escape \
    src/runtime/binary_object/atoms.rs \
    'fn detach(value: ArchivedBytecodeImage) { let _ = value.archived_image_shared_backings; }'
expect_rejected sab-image-extra-impl sab-image-aggregate-escape \
    src/runtime/binary_object/graph/sab_transport.rs \
    'impl ArchivedBytecodeImage { fn leak(&self) -> &BytecodeImage { &self.archived_image_payload } }'
expect_rejected sab-image-trait-impl sab-image-aggregate-escape \
    src/runtime/binary_object/graph/sab_transport.rs \
    'impl LeakArchive for ArchivedBytecodeImage { fn leak(self) -> BytecodeImage { self.archived_image_payload } }'
expect_rejected sab-image-raw-alias-literal sab-image-field-escape \
    src/runtime/binary_object/graph/sab_transport.rs \
    'type r#Archive = ArchivedBytecodeImage; fn forge(image: BytecodeImage, table: Box<[SharedBackingDescriptor]>) { let _ = r#Archive { archived_image_payload: image, archived_image_shared_backings: table }; }'
expect_rewrite_rejected sab-image-finalizer-wrapper sab-transport-entrypoint-body \
    src/runtime/binary_object/graph/sab_transport.rs \
    '    cursor.finish_bytecode_image(image).map_err(Into::into)' \
    '    audit_finalize_image(cursor, image).map_err(Into::into)'
expect_rejected sab-extra-transport-function sab-transport-free-function-set \
    src/runtime/binary_object/graph/sab_transport.rs \
    'fn audit_finalize_image() {}'
expect_rejected sab-unicode-transport-function sab-transport-free-function-set \
    src/runtime/binary_object/graph/sab_transport.rs \
    'pub(in crate::runtime::binary_object) fn 完成图归档() {}'
expect_rejected sab-unicode-finalizer-wrapper sab-archive-call-site-set \
    src/runtime/binary_object/graph/sab_transport.rs \
    'pub(in crate::runtime::binary_object) fn 完成图归档(cursor: SabTransportCursor<'"'"'_>, graph: WireGraph) -> Result<ArchivedWireGraph, SabArchiveError> { cursor.finish_graph_archive(graph) }'
expect_rejected sab-finalizer-const-wrapper sab-archive-call-site-set \
    src/runtime/binary_object/graph/sab_transport.rs \
    'pub(in crate::runtime::binary_object) const AUDIT_FINALIZE_GRAPH: for<'"'"'a> fn(SabTransportCursor<'"'"'a>, WireGraph) -> Result<ArchivedWireGraph, SabArchiveError> = |cursor, graph| cursor.finish_graph_archive(graph);'
expect_rejected sab-extra-top-level-const sab-transport-top-level-item-set \
    src/runtime/binary_object/graph/sab_transport.rs \
    'pub(in crate::runtime::binary_object) const AUDIT_ITEM: usize = 0;'
expect_rewrite_rejected sab-associated-finalizer-const sab-archive-call-site-set \
    src/runtime/binary_object/graph/sab_transport.rs \
    '    fn finish_bytecode_image(self, image: BytecodeImage) -> Result<ArchivedBytecodeImage, Error> {' \
    '    const 审计图终结器: for<'"'"'b> fn(SabTransportCursor<'"'"'b>, WireGraph) -> Result<ArchivedWireGraph, SabArchiveError> = |cursor, graph| cursor.finish_graph_archive(graph); fn finish_bytecode_image(self, image: BytecodeImage) -> Result<ArchivedBytecodeImage, Error> {'
expect_rewrite_rejected sab-unicode-cursor-method sab-cursor-method-set \
    src/runtime/binary_object/graph/sab_transport.rs \
    '    fn finish_bytecode_image(self, image: BytecodeImage) -> Result<ArchivedBytecodeImage, Error> {' \
    '    fn 完成图归档(&self) {} fn finish_bytecode_image(self, image: BytecodeImage) -> Result<ArchivedBytecodeImage, Error> {'
expect_rewrite_rejected sab-public-build-cursor sab-transport-private-member \
    src/runtime/binary_object/graph/sab_transport.rs \
    '    fn build_cursor(' \
    '    pub(in crate::runtime::binary_object) fn build_cursor('
expect_rewrite_rejected sab-public-graph-finalizer sab-transport-private-member \
    src/runtime/binary_object/graph/sab_transport.rs \
    '    fn finish_graph_archive(self, graph: WireGraph) -> Result<ArchivedWireGraph, Error> {' \
    '    pub(in crate::runtime::binary_object) fn finish_graph_archive(self, graph: WireGraph) -> Result<ArchivedWireGraph, Error> {'
expect_rewrite_rejected sab-public-image-finalizer sab-transport-private-member \
    src/runtime/binary_object/graph/sab_transport.rs \
    '    fn finish_bytecode_image(self, image: BytecodeImage) -> Result<ArchivedBytecodeImage, Error> {' \
    '    pub(in crate::runtime::binary_object) fn finish_bytecode_image(self, image: BytecodeImage) -> Result<ArchivedBytecodeImage, Error> {'
expect_rewrite_rejected sab-public-cursor-field sab-cursor-shape \
    src/runtime/binary_object/graph/sab_transport.rs \
    '    cursor_next_occurrence: usize,' \
    '    pub(in crate::runtime::binary_object) cursor_next_occurrence: usize,'
expect_rewrite_rejected sab-public-input-field sab-input-shape \
    src/runtime/binary_object/graph/sab_transport.rs \
    '    transport_wire_bytes: &'"'"'a [u8],' \
    '    pub(in crate::runtime::binary_object) transport_wire_bytes: &'"'"'a [u8],'
expect_rewrite_rejected sab-public-image-field sab-image-aggregate-shape \
    src/runtime/binary_object/graph/sab_transport.rs \
    '    archived_image_payload: BytecodeImage,' \
    '    pub(in crate::runtime::binary_object) archived_image_payload: BytecodeImage,'

echo "binary-object production boundary passed; all isolation canaries were rejected"
