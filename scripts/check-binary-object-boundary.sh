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
from pathlib import Path
import hashlib
import re
import sys


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


lib_source = read_source("src/lib.rs")
for match in re.finditer(r"\bbinary_object\b", lib_source):
    fail(
        "public-lib-boundary",
        "src/lib.rs must not name binary_object; found "
        + location("src/lib.rs", lib_source, match.start()),
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
    "DetachedPrimitive",
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
    "src/runtime/binary_object/function_translate/dto.rs": 40,
    "src/runtime/binary_object/function_translate/mod.rs": 5,
    "src/runtime/binary_object/graph/arena.rs": 19,
    "src/runtime/binary_object/graph/decode.rs": 28,
    "src/runtime/binary_object/graph/encode.rs": 4,
    "src/runtime/binary_object/graph/mod.rs": 6,
    "src/runtime/binary_object/graph/model.rs": 56,
    "src/runtime/binary_object/graph/sab_transport.rs": 38,
    "src/runtime/binary_object/graph/write_state.rs": 21,
    "src/runtime/binary_object/mod.rs": 2,
    "src/runtime/binary_object/ordinary_leaf.rs": 27,
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
    "src/runtime/binary_object/function_translate/dto.rs": 40,
    "src/runtime/binary_object/function_translate/mod.rs": 5,
    "src/runtime/binary_object/mod.rs": 2,
    "src/runtime/binary_object/ordinary_leaf.rs": 27,
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
    PushI32 PushConstant PushAtom PushUndefined PushNull PushFalse PushTrue
    PushBigIntI32 PushEmptyString Stack Unary PostDec PostInc GetLocal PutLocal
    SetLocal GetArgument PutArgument SetArgument Binary Predicate IfFalse IfTrue
    Goto Call Construct CallMethod ArrayFrom Return ReturnUndefined
""".split()
invocation_variant_names = ("Call", "Construct", "CallMethod", "ArrayFrom")
recipe_invocation_shapes = {
    name: (
        len(re.findall(rf"\b{name}[ \t\n]*,", recipe_code)),
        bool(re.search(rf"\b{name}[ \t\n]*[({{]", recipe_code)),
    )
    for name in invocation_variant_names
}
if enum_variant_names(recipe_code) != expected_recipe_variants or any(
    recipe_invocation_shapes[name] != (1, False) for name in invocation_variant_names
):
    fail(
        "function-translate-recipe-shape",
        "Recipe must retain the exact reviewed inventory with unit Call, Construct, CallMethod, and ArrayFrom recipes; "
        f"found {enum_variant_names(recipe_code)} with invocation shapes {recipe_invocation_shapes}",
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
    Blocked OutsideTarget PushI32 PushConstant PushAtom PushUndefined PushNull
    PushBool PushBigIntI32 PushEmptyString Stack Unary PostDec PostInc GetLocal
    PutLocal SetLocal GetArgument PutArgument SetArgument Binary Predicate IfFalse
    IfTrue Goto Call Construct CallMethod ArrayFrom Return ReturnUndefined
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
    or any(
        function_invocation_payloads[name] != ["u16"]
        for name in invocation_variant_names
    )
):
    fail(
        "function-translate-dto-shape",
        "FunctionOp must retain the exact reviewed inventory with u16 invocation payloads; "
        f"found {enum_variant_names(dto_function_op_code)} with invocation payloads {function_invocation_payloads}",
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
    ("Blocked", "ScalarOnly", "OrdinaryOnly", "Shared"), (120, 1, 94, 29)
))
derived_registry_counts = (
    registry_audience_counts["ScalarOnly"] + registry_audience_counts["Shared"],
    registry_audience_counts["OrdinaryOnly"] + registry_audience_counts["Shared"],
    244 - registry_audience_counts["Blocked"],
)
if (
    registry_audience_counts != expected_registry_audience_counts
    or derived_registry_counts != (30, 123, 124)
):
    fail(
        "function-translate-registry-audience",
        "the centralized registry must preserve the exact final stage-three-A physical cohorts; "
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
expect_admitted("OrdinaryOnly", "Recipe::Construct", (33,))
expect_admitted("OrdinaryOnly", "Recipe::CallMethod", (36,))
expect_admitted("OrdinaryOnly", "Recipe::ArrayFrom", (38,))
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
    33: (("call_constructor", 3, 2, 1, "NPop"), "OrdinaryOnly", "Recipe::Construct"),
    34: (("call", 3, 1, 1, "NPop"), "OrdinaryOnly", "Recipe::Call"),
    35: (("tail_call", 3, 1, 0, "NPop"), "Blocked", "Invocation"),
    36: (("call_method", 3, 2, 1, "NPop"), "OrdinaryOnly", "Recipe::CallMethod"),
    37: (("tail_call_method", 3, 2, 0, "NPop"), "Blocked", "Invocation"),
    38: (("array_from", 3, 0, 1, "NPop"), "OrdinaryOnly", "Recipe::ArrayFrom"),
    39: (("apply", 3, 3, 1, "U16"), "Blocked", "Invocation"),
    41: (("return_undef", 1, 0, 0, "None"), "OrdinaryOnly", "Recipe::ReturnUndefined"),
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
        "reviewed invocation, zero-stack return-undefined, and plain-call boundaries must retain their pinned descriptors and policy rows; "
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

stage_three_a_blocked_rows = (
    (35, "NPop", "Invocation"),
    (37, "NPop", "Invocation"),
    (39, "U16", "Invocation"),
)
found_stage_three_a_blocked_rows = tuple(
    (raw, registry_rows[raw][1], registry_rows[raw][3])
    for raw, _, _ in stage_three_a_blocked_rows
    if registry_rows[raw][2] == "Blocked"
)
if found_stage_three_a_blocked_rows != stage_three_a_blocked_rows:
    fail(
        "function-translate-stage-three-a-frontier",
        "tail-call raws 35 and 37 plus apply raw 39 must remain blocked in Invocation; "
        f"found {found_stage_three_a_blocked_rows}",
    )

blocker_count_tokens = """
    InvalidSentinel 1 ValueConstruction 8 FunctionGraph 2 Invocation 3 Completion 1
    Exception 2 EvalOrModule 3 Binding 7 Property 16 ObjectConstruction 15
    LexicalEnvironment 25 ControlFlow 4 DynamicScope 9 Iteration 11 Suspension 5
    Operator 4 Specialized 4
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
        "the blocked frontier must retain all 17 nonempty typed categories and their exact counts; "
        f"found {found_blocker_counts} with unexpected {unexpected_blockers}",
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
        "TranslationBlocker must retain the 17 reviewed ordered categories and no retired empty bucket; "
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
Recipe::Construct @ NativeOperands::NPop(argument_count) @ { ready(FunctionOp::Construct(*argument_count)) }
Recipe::CallMethod @ NativeOperands::NPop(argument_count) @ { ready(FunctionOp::CallMethod(*argument_count)) }
Recipe::ArrayFrom @ NativeOperands::NPop(argument_count) @ { ready(FunctionOp::ArrayFrom(*argument_count)) }
Recipe::Return @ NativeOperands::None @ ready(FunctionOp::Return)
Recipe::ReturnUndefined @ NativeOperands::None @ ready(FunctionOp::ReturnUndefined)
""".strip().splitlines()
expected_single_step_arms = [
    tuple(row.split(" @ ", 2)) for row in single_step_rows
]
if len(lowering_arm_matches) != 46 or found_single_step_arms != expected_single_step_arms:
    fail(
        "function-translate-semantic-dispatch",
        "lower_operation must retain all 46 reviewed Recipe/operand arms and each exact normalized RHS payload expression; "
        f"found {found_single_step_arms}",
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
        enum:DetachedPrimitive enum:OrdinaryLeafOp enum:OrdinaryLeafStackOp
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
    enum:DetachedPrimitive enum:OrdinaryLeafOp enum:OrdinaryLeafStackOp
    enum:OrdinaryLeafUnaryOp enum:OrdinaryLeafBinaryOp enum:OrdinaryLeafPredicateOp
    struct:OrdinaryLeafDraft enum:OrdinaryLeafReadError struct:AdmissionLimits
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
    PushI32 PushConst PushUndefined PushNull PushBool PushBigIntI32 PushEmptyString
    Stack Unary PostDec PostInc GetLocal PutLocal SetLocal GetArgument PutArgument
    SetArgument Binary Predicate IfFalse IfTrue Goto Call Construct CallMethod
    ArrayFrom Return ReturnUndefined
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
    or any(
        ordinary_invocation_payloads[name] != ["u16"]
        for name in invocation_variant_names
    )
):
    fail(
        "ordinary-leaf-operation-shape",
        "OrdinaryLeafOp must retain the exact reviewed inventory with u16 invocation payloads; "
        f"found {enum_variant_names(ordinary_leaf_op_code)} with invocation payloads {ordinary_invocation_payloads}",
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
    copy_wire_string copy_bigint lower_code lower_operation lower_constant lower_local
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
ordinary_handoff_rows = """
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
FunctionOp::Construct(argument_count) @ Ok(OrdinaryLeafOp::Construct(*argument_count))
FunctionOp::CallMethod(argument_count) @ Ok(OrdinaryLeafOp::CallMethod(*argument_count))
FunctionOp::ArrayFrom(element_count) @ Ok(OrdinaryLeafOp::ArrayFrom(*element_count))
FunctionOp::Return @ Ok(OrdinaryLeafOp::Return)
FunctionOp::ReturnUndefined @ Ok(OrdinaryLeafOp::ReturnUndefined)
""".strip().splitlines()
expected_ordinary_handoff = [tuple(row.split(" @ ", 1)) for row in ordinary_handoff_rows]
found_ordinary_handoff = rustfmt_match_arms(ordinary_lower_operation, "FunctionOp::")
if found_ordinary_handoff != expected_ordinary_handoff:
    fail(
        "ordinary-leaf-translated-code",
        "all 28 sanitized operations must retain their exact ordinary-leaf payload and typed-family mapping; "
        f"found {found_ordinary_handoff}",
    )
if (
    len(re.findall(r"\binstruction[ \t\n]*\.[ \t\n]*supports_ordinary[ \t\n]*\(", ordinary_lower_code)) != 1
    or len(re.findall(r"\binstruction[ \t\n]*\.[ \t\n]*operation[ \t\n]*\(", ordinary_lower_code)) != 1
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
        PushI32 PushConst PushUndefined PushNull PushBool PushBool PushBigIntI32
        PushEmptyString Stack Unary PostDec PostInc GetLocal PutLocal SetLocal
        GetArgument PutArgument SetArgument Binary Predicate IfFalse IfTrue Goto Call
        Construct CallMethod ArrayFrom Return ReturnUndefined
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
OrdinaryLeafOp::Construct(argument_count) @ Instruction::Construct(argument_count)
OrdinaryLeafOp::CallMethod(argument_count) @ Instruction::CallMethod(argument_count)
OrdinaryLeafOp::ArrayFrom(element_count) @ Instruction::ArrayFrom(element_count)
OrdinaryLeafOp::Return @ Instruction::Return
OrdinaryLeafOp::ReturnUndefined @ Instruction::ReturnUndefined
""".strip().splitlines()
    excluded_publisher_arms = {
        *(f"OrdinaryLeafOp::{family}(operation)" for family in publisher_families),
        "OrdinaryLeafOp::PushBigIntI32(_) | OrdinaryLeafOp::PushEmptyString",
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
    synthetic_index_fragments = (
        "let index = *next_synthetic_index;",
        "*next_synthetic_index = next_synthetic_index.checked_add(1)",
        "Instruction::PushConst(index)",
    )
    synthetic_index_offsets = [
        normalized_synthetic_index.find(fragment) for fragment in synthetic_index_fragments
    ]
    if (
        len(found_publisher_arms) != 28
        or found_publisher_direct != expected_publisher_direct
        or published_variants != expected_published_variants
        or any(
        normalized_instruction_lowering.count(fragment) != 1
        for fragment in (
            "OrdinaryLeafOp::PushBigIntI32(_) | OrdinaryLeafOp::PushEmptyString =>",
        )
        )
        or normalized_instruction_lowering.count("Instruction::PushConst(index)") != 2
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
            "ordinary operations must retain their complete typed publisher mapping and stable synthetic indices",
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

bytecode_code = rust_code_only(read_source("src/bytecode.rs"))
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

src_root = root / "src"
if src_root.is_symlink() or not src_root.is_dir():
    fail("missing-source", "src must be a regular directory")
    production_sources: list[Path] = []
else:
    production_sources = sorted(src_root.rglob("*.rs"))

facade_name_pattern = re.compile(
    r"\b(?:ScalarValueDraft|ScalarUnaryOp|ScalarScriptReadError|ScalarStringDraft|"
    r"decode_trusted_scalar_script|DetachedPrimitive|OrdinaryLeafDraft|"
    r"OrdinaryLeafMetadataDraft|OrdinaryLeafOp|OrdinaryLeafReadError|"
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

case ${1:-} in
    "") ;;
    --scan-only)
        [[ $# == 2 ]] || die "usage: $0 --scan-only ROOT"
        scan_root "$2"
        exit 0
        ;;
    *) die "usage: $0 [--scan-only ROOT]" ;;
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
    'pub(super) use ordinary_leaf::{DetachedPrimitive, OrdinaryLeafBinaryOp, OrdinaryLeafDraft, OrdinaryLeafMetadataDraft, OrdinaryLeafOp, OrdinaryLeafPredicateOp, OrdinaryLeafReadError, OrdinaryLeafStackOp, OrdinaryLeafUnaryOp, RootFunctionConstantSelector, decode_trusted_ordinary_leaf};' \
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
    local case_root=$tmp_dir/$label
    local output=$case_root.output

    mkdir -p "$case_root"
    cp -R "$repository_root/src" "$case_root/src"
    python3 - "$case_root/$relative" "$before" "$after" <<'PY'
from pathlib import Path
import sys

path = Path(sys.argv[1])
before = sys.argv[2]
after = sys.argv[3]
source = path.read_text(encoding="utf-8")
if source.count(before) != 1:
    raise SystemExit(f"full rewrite canary expected one occurrence of {before!r}")
path.write_text(source.replace(before, after), encoding="utf-8")
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
    'DetachedPrimitive, OrdinaryLeafBinaryOp, OrdinaryLeafDraft,' \
    'BytecodeImage, DetachedPrimitive, OrdinaryLeafBinaryOp, OrdinaryLeafDraft,'
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
stage3a-tail-call-admission|function-translate-registry-audience|src/runtime/binary_object/function_translate/capability.rs|    row!(35, NPop, Blocked, Invocation),|    row!(35, NPop, OrdinaryOnly, Recipe::Construct),
stage3a-tail-call-method-admission|function-translate-registry-audience|src/runtime/binary_object/function_translate/capability.rs|    row!(37, NPop, Blocked, Invocation),|    row!(37, NPop, OrdinaryOnly, Recipe::CallMethod),
stage3a-apply-admission|function-translate-registry-audience|src/runtime/binary_object/function_translate/capability.rs|    row!(39, U16, Blocked, Invocation),|    row!(39, U16, OrdinaryOnly, Recipe::ArrayFrom),
translate-recipe-call-payload|function-translate-recipe-shape|src/runtime/binary_object/function_translate/capability.rs|    Call,|    Call(u16),
translate-dto-call-payload|function-translate-dto-shape|src/runtime/binary_object/function_translate/dto.rs|    Call(u16),|    Call(u32),
ordinary-call-dto-payload|ordinary-leaf-operation-shape|src/runtime/binary_object/ordinary_leaf.rs|    Call(u16),|    Call(u32),
translate-blocker-bucket-revival|function-translate-dto-shape|src/runtime/binary_object/function_translate/dto.rs|    FunctionGraph,\n    Invocation,|    FunctionGraph,\n    StackManipulation,\n    Invocation,
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
stage3a-ordinary-array-from-count-minus-one|ordinary-leaf-translated-code|src/runtime/binary_object/ordinary_leaf.rs|        FunctionOp::ArrayFrom(element_count) => Ok(OrdinaryLeafOp::ArrayFrom(*element_count)),|        FunctionOp::ArrayFrom(element_count) => Ok(OrdinaryLeafOp::ArrayFrom(element_count.saturating_sub(1))),
stage3a-ordinary-call-method-to-call|ordinary-leaf-translated-code|src/runtime/binary_object/ordinary_leaf.rs|        FunctionOp::CallMethod(argument_count) => Ok(OrdinaryLeafOp::CallMethod(*argument_count)),|        FunctionOp::CallMethod(argument_count) => Ok(OrdinaryLeafOp::Call(*argument_count)),
ordinary-publisher-get-local-offset|ordinary-leaf-consumer-lowering|src/runtime/binary_object_publish.rs|        OrdinaryLeafOp::GetLocal(index) => Instruction::GetLocal(index),|        OrdinaryLeafOp::GetLocal(index) => Instruction::GetLocal(index.saturating_add(1)),
ordinary-publisher-call-argc|ordinary-leaf-consumer-lowering|src/runtime/binary_object_publish.rs|        OrdinaryLeafOp::Call(argument_count) => Instruction::Call(argument_count),|        OrdinaryLeafOp::Call(argument_count) => Instruction::Call(argument_count.saturating_add(1)),
ordinary-publisher-call-method|ordinary-leaf-consumer-lowering|src/runtime/binary_object_publish.rs|        OrdinaryLeafOp::Call(argument_count) => Instruction::Call(argument_count),|        OrdinaryLeafOp::Call(argument_count) => Instruction::CallMethod(argument_count),
ordinary-publisher-call-push-undefined|ordinary-leaf-consumer-lowering|src/runtime/binary_object_publish.rs|        OrdinaryLeafOp::Call(argument_count) => Instruction::Call(argument_count),|        OrdinaryLeafOp::Call(_argument_count) => Instruction::Undefined,
stage3a-publisher-array-from-wrong-op|ordinary-leaf-consumer-lowering|src/runtime/binary_object_publish.rs|        OrdinaryLeafOp::ArrayFrom(element_count) => Instruction::ArrayFrom(element_count),|        OrdinaryLeafOp::ArrayFrom(element_count) => Instruction::Construct(element_count),
ordinary-synthetic-index-offset|ordinary-leaf-consumer-lowering|src/runtime/binary_object_publish.rs|            let index = *next_synthetic_index;|            let index = next_synthetic_index.saturating_add(1);
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
