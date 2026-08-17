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

    item = matches[0]
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
    "bytecode_image",
    "graph",
    "pinned_atoms",
    "pinned_opcodes",
    "read_cursor",
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
    "ScalarScriptDraft",
    "ScalarScriptReadError",
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

facade_offsets = {match.start() for match in scalar_facades}
for match in public_use_pattern.finditer(binary_root_code):
    if match.start() in facade_offsets:
        continue
    fail(
        "root-reexport",
        "binary_object root may re-export only the reviewed scalar-script facade; found "
        + location(binary_root_relative, binary_root_source, match.start()),
    )

image_root_relative = "src/runtime/binary_object/bytecode_image/mod.rs"
image_root_source = read_source(image_root_relative)
image_root_code = rust_code_only(image_root_source)
for module in ("atoms", "budget", "decode", "encode", "model"):
    declarations = re.findall(
        rf"(?m)^[ \t]*mod[ \t]+{re.escape(module)}[ \t]*;[ \t]*$",
        image_root_code,
    )
    if len(declarations) != 1:
        fail(
            "image-private-module",
            f"{image_root_relative} must contain exactly one private `mod {module};` declaration",
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
scalar_draft_pattern = re.compile(
    rf"{scalar_noncopy_derive}[ \t\n]*{scalar_visibility}"
    r"[ \t\n]+enum[ \t\n]+ScalarScriptDraft[ \t\n]*\{"
    r"[ \t\n]*Undefined[ \t\n]*,"
    r"[ \t\n]*Null[ \t\n]*,"
    r"[ \t\n]*Bool[ \t\n]*\([ \t\n]*bool[ \t\n]*\)[ \t\n]*,"
    r"[ \t\n]*Int[ \t\n]*\([ \t\n]*i32[ \t\n]*\)[ \t\n]*,"
    r"[ \t\n]*Float64Bits[ \t\n]*\([ \t\n]*u64[ \t\n]*\)[ \t\n]*,"
    r"[ \t\n]*BigIntI32[ \t\n]*\([ \t\n]*i32[ \t\n]*\)[ \t\n]*,"
    r"[ \t\n]*BigIntBytes[ \t\n]*\([ \t\n]*Box[ \t\n]*<"
    r"[ \t\n]*\[[ \t\n]*u8[ \t\n]*\][ \t\n]*>[ \t\n]*\)[ \t\n]*,"
    r"[ \t\n]*NegatedBigIntI32[ \t\n]*\([ \t\n]*i32[ \t\n]*\)[ \t\n]*,"
    r"[ \t\n]*NegatedBigIntBytes[ \t\n]*\([ \t\n]*Box[ \t\n]*<"
    r"[ \t\n]*\[[ \t\n]*u8[ \t\n]*\][ \t\n]*>[ \t\n]*\)[ \t\n]*,"
    r"[ \t\n]*EmptyString[ \t\n]*,?[ \t\n]*\}"
)
if len(scalar_draft_pattern.findall(scalar_script_code)) != 1:
    fail(
        "scalar-script-draft-shape",
        "ScalarScriptDraft must be non-Copy, runtime-visible only, and contain exactly the reviewed plain scalars plus strongly typed NegatedBigIntI32 and NegatedBigIntBytes variants",
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
    r"[ \t\n]*ScalarScriptDraft[ \t\n]*,[ \t\n]*ScalarScriptReadError[ \t\n]*>"
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
    scalar_script_code,
)
scalar_opcode_entries = re.findall(
    r"(?m)^[ \t]*const[ \t]+(OP_[A-Z0-9_]+)[ \t]*:[ \t]*u8[ \t]*=[ \t]*(0x[0-9a-fA-F]+)[ \t]*;[ \t]*$",
    scalar_script_code,
)
scalar_opcode_constants = {name: int(raw, 16) for name, raw in scalar_opcode_entries}
expected_scalar_opcode_constants = {
    "OP_PUSH_I32": 0x01,
    "OP_PUSH_CONST": 0x02,
    "OP_UNDEFINED": 0x06,
    "OP_NULL": 0x07,
    "OP_PUSH_FALSE": 0x09,
    "OP_PUSH_TRUE": 0x0A,
    "OP_RETURN": 0x28,
    "OP_NEG": 0x8A,
    "OP_PUSH_BIGINT_I32": 0xB0,
    "OP_PUSH_MINUS1": 0xB2,
    "OP_PUSH_0": 0xB3,
    "OP_PUSH_7": 0xBA,
    "OP_PUSH_I8": 0xBB,
    "OP_PUSH_I16": 0xBC,
    "OP_PUSH_CONST8": 0xBD,
    "OP_PUSH_EMPTY_STRING": 0xBF,
    "OP_SET_LOC0": 0xCB,
}
if (
    sorted(scalar_opcode_declarations) != sorted(expected_scalar_opcode_constants)
    or len(scalar_opcode_declarations) != len(expected_scalar_opcode_constants)
    or len(scalar_opcode_entries) != len(expected_scalar_opcode_constants)
    or scalar_opcode_constants != expected_scalar_opcode_constants
):
    fail(
        "scalar-script-opcode-set",
        "scalar-script admission must define each reviewed scalar push, the one BigInt-only neg, set_loc0, and return opcode exactly once; "
        f"found declarations {scalar_opcode_declarations} and exact entries {scalar_opcode_entries}",
    )

scalar_push_pattern = re.compile(
    rf"{scalar_noncopy_derive}[ \t\n]*enum[ \t\n]+ScalarPush[ \t\n]*\{{"
    r"[ \t\n]*Direct[ \t\n]*\([ \t\n]*ScalarScriptDraft[ \t\n]*\)"
    r"[ \t\n]*,[ \t\n]*Constant[ \t\n]*\([ \t\n]*u32[ \t\n]*\)"
    r"[ \t\n]*,?[ \t\n]*\}"
)
if len(scalar_push_pattern.findall(scalar_script_code)) != 1:
    fail(
        "scalar-script-push-shape",
        "ScalarPush must remain one private Direct(ScalarScriptDraft) or Constant(u32) discriminator",
    )

bigint_push_pattern = re.compile(
    rf"{scalar_noncopy_derive}[ \t\n]*enum[ \t\n]+BigIntPush[ \t\n]*\{{"
    r"[ \t\n]*DirectI32[ \t\n]*\([ \t\n]*i32[ \t\n]*\)"
    r"[ \t\n]*,[ \t\n]*Constant[ \t\n]*\([ \t\n]*u32[ \t\n]*\)"
    r"[ \t\n]*,?[ \t\n]*\}"
)
if len(bigint_push_pattern.findall(scalar_script_code)) != 1:
    fail(
        "scalar-script-bigint-push-shape",
        "BigIntPush must remain one private DirectI32(i32) or Constant(u32) discriminator",
    )

scalar_sequence_pattern = re.compile(
    rf"{scalar_noncopy_derive}[ \t\n]*enum[ \t\n]+ScalarSequence[ \t\n]*\{{"
    r"[ \t\n]*Plain[ \t\n]*\{[ \t\n]*push[ \t\n]*:[ \t\n]*ScalarPush"
    r"[ \t\n]*,[ \t\n]*width[ \t\n]*:[ \t\n]*u32[ \t\n]*\}"
    r"[ \t\n]*,[ \t\n]*NegatedBigInt[ \t\n]*\{"
    r"[ \t\n]*push[ \t\n]*:[ \t\n]*BigIntPush[ \t\n]*,"
    r"[ \t\n]*width[ \t\n]*:[ \t\n]*u32[ \t\n]*\}"
    r"[ \t\n]*,?[ \t\n]*\}"
)
if len(scalar_sequence_pattern.findall(scalar_script_code)) != 1:
    fail(
        "scalar-script-sequence-shape",
        "ScalarSequence must keep plain scalars separate from the strongly typed single-neg BigInt shape",
    )

scalar_copy_pattern = re.compile(
    r"#[ \t\n]*\[[ \t\n]*derive[ \t\n]*\([^\]]*\bCopy\b[^\]]*\)"
    r"[ \t\n]*\](?:[ \t\n]*#[ \t\n]*\[[^\]]*\])*[ \t\n]*"
    rf"(?:{scalar_visibility}[ \t\n]+)?enum[ \t\n]+"
    r"(?:ScalarScriptDraft|ScalarPush|BigIntPush|ScalarSequence)\b|"
    r"\bimpl[ \t\n]+Copy[ \t\n]+for[ \t\n]+"
    r"(?:ScalarScriptDraft|ScalarPush|BigIntPush|ScalarSequence)\b"
)
if scalar_copy_pattern.search(scalar_script_code):
    fail(
        "scalar-script-draft-shape",
        "the scalar draft, push, and sequence discriminators must not regain Copy semantics around owned BigInt bytes",
    )

scalar_sequence_decoder_item_pattern = re.compile(
    r"\bfn[ \t\n]+decode_scalar_sequence[ \t\n]*\("
    r"[ \t\n]*bytes[ \t\n]*:[ \t\n]*&[ \t\n]*\[u8\][ \t\n]*\)"
    r"[ \t\n]*->[ \t\n]*Option[ \t\n]*<[ \t\n]*ScalarSequence"
    r"[ \t\n]*>[ \t\n]*\{",
    re.DOTALL,
)
scalar_sequence_decoder_code, _, _ = unique_braced_item(
    scalar_script_code,
    scalar_sequence_decoder_item_pattern,
    "scalar-script-sequence-decoder",
    "private &[u8] to ScalarSequence decoder",
)
if scalar_sequence_decoder_code:
    expected_sequence_decoder_source = """
        fn decode_scalar_sequence(bytes: &[u8]) -> Option<ScalarSequence> {
            match bytes {
                [OP_PUSH_CONST8, index, OP_NEG, OP_SET_LOC0, OP_RETURN] => {
                    Some(ScalarSequence::NegatedBigInt {
                        push: BigIntPush::Constant(u32::from(*index)),
                        width: 2,
                    })
                }
                [
                    OP_PUSH_CONST,
                    byte_0,
                    byte_1,
                    byte_2,
                    byte_3,
                    OP_NEG,
                    OP_SET_LOC0,
                    OP_RETURN,
                ] => Some(ScalarSequence::NegatedBigInt {
                    push: BigIntPush::Constant(u32::from_le_bytes([*byte_0, *byte_1, *byte_2, *byte_3])),
                    width: 5,
                }),
                [
                    OP_PUSH_BIGINT_I32,
                    byte_0,
                    byte_1,
                    byte_2,
                    byte_3,
                    OP_NEG,
                    OP_SET_LOC0,
                    OP_RETURN,
                ] => Some(ScalarSequence::NegatedBigInt {
                    push: BigIntPush::DirectI32(i32::from_le_bytes([*byte_0, *byte_1, *byte_2, *byte_3])),
                    width: 5,
                }),
                _ => decode_scalar_push(bytes).map(|(push, width)| ScalarSequence::Plain { push, width }),
            }
        }
    """
    if (
        " ".join(scalar_sequence_decoder_code.split())
        != " ".join(rust_code_only(expected_sequence_decoder_source).split())
    ):
        fail(
            "scalar-script-sequence-decoder",
            "decode_scalar_sequence must admit exactly one OP_NEG only after a direct-i32 or index-zero-capable BigInt push spelling, before set_loc0 and return",
        )

scalar_push_decoder_item_pattern = re.compile(
    r"\bfn[ \t\n]+decode_scalar_push[ \t\n]*\("
    r"[ \t\n]*bytes[ \t\n]*:[ \t\n]*&[ \t\n]*\[u8\][ \t\n]*\)"
    r"[ \t\n]*->[ \t\n]*Option[ \t\n]*<[ \t\n]*\("
    r"[ \t\n]*ScalarPush[ \t\n]*,[ \t\n]*u32[ \t\n]*\)"
    r"[ \t\n]*>[ \t\n]*\{",
    re.DOTALL,
)
scalar_push_decoder_code, _, _ = unique_braced_item(
    scalar_script_code,
    scalar_push_decoder_item_pattern,
    "scalar-script-push-decoder",
    "private &[u8] to (ScalarPush, u32) decoder",
)

if scalar_push_decoder_code:
    normalized_push_decoder = " ".join(scalar_push_decoder_code.split())
    expected_push_decoder = (
        "fn decode_scalar_push(bytes: &[u8]) -> Option<(ScalarPush, u32)> { "
        "match bytes { "
        "[OP_PUSH_CONST8, index, OP_SET_LOC0, OP_RETURN] => { "
        "Some((ScalarPush::Constant(u32::from(*index)), 2)) } "
        "[ OP_PUSH_CONST, byte_0, byte_1, byte_2, byte_3, OP_SET_LOC0, "
        "OP_RETURN, ] => Some(( "
        "ScalarPush::Constant(u32::from_le_bytes([*byte_0, *byte_1, "
        "*byte_2, *byte_3])), 5, )), "
        "_ => decode_direct_scalar(bytes).map(|(draft, width)| "
        "(ScalarPush::Direct(draft), width)), } }"
    )
    if normalized_push_decoder != expected_push_decoder:
        fail(
            "scalar-script-push-decoder",
            "decode_scalar_push must admit only push_const8/u8 and push_const/little-endian-u32 before the direct scalar decoder",
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
    ("pub(in crate::runtime)", "enum", "ScalarScriptDraft"),
    ("pub(in crate::runtime)", "enum", "ScalarScriptReadError"),
    ("pub(in crate::runtime)", "fn", "decode_trusted_scalar_script"),
]
if sorted(scalar_visible_items) != sorted(expected_scalar_visible_items):
    fail(
        "scalar-script-visible-item-set",
        "scalar_script.rs may expose only the reviewed draft, error, and decoder to runtime; "
        f"found {scalar_visible_items}",
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

scalar_admission_item_pattern = re.compile(
    r"\bfn[ \t\n]+admit_image[ \t\n]*\([^{};]*\)"
    r"[ \t\n]*->[^{;]+\{",
    re.DOTALL,
)
scalar_admission_code, scalar_admission_start, _ = unique_braced_item(
    scalar_script_code,
    scalar_admission_item_pattern,
    "scalar-script-admission-empty-boundaries",
    "private admit_image function",
)

scalar_admission_empty_boundaries = (
    (
        "the original input atom-slot count",
        re.compile(
            r"\bif[ \t\n]+image[ \t\n]*\.[ \t\n]*input_atom_slot_count"
            r"[ \t\n]*\([ \t\n]*\)[ \t\n]*!=[ \t\n]*0[ \t\n]*\{"
            r"[ \t\n]*return[ \t\n]+unadmitted[ \t\n]*\([ \t\n]*\)"
            r"[ \t\n]*;[ \t\n]*\}"
        ),
        re.compile(
            r"\bimage[ \t\n]*\.[ \t\n]*input_atom_slot_count"
            r"[ \t\n]*\([ \t\n]*\)"
        ),
    ),
    (
        "the semantic dynamic-atom table",
        re.compile(
            r"\bif[ \t\n]+![ \t\n]*image[ \t\n]*\.[ \t\n]*atoms"
            r"[ \t\n]*\([ \t\n]*\)[ \t\n]*\.[ \t\n]*is_empty"
            r"[ \t\n]*\([ \t\n]*\)[ \t\n]*\{"
            r"[ \t\n]*return[ \t\n]+unadmitted[ \t\n]*\([ \t\n]*\)"
            r"[ \t\n]*;[ \t\n]*\}"
        ),
        re.compile(
            r"\bimage[ \t\n]*\.[ \t\n]*atoms[ \t\n]*\([ \t\n]*\)"
        ),
    ),
    (
        "the native payload atom-relocation table",
        re.compile(
            r"\bif[ \t\n]+![ \t\n]*native_payload[ \t\n]*\.[ \t\n]*atom_relocations"
            r"[ \t\n]*\([ \t\n]*\)[ \t\n]*\.[ \t\n]*is_empty"
            r"[ \t\n]*\([ \t\n]*\)[ \t\n]*\{"
            r"[ \t\n]*return[ \t\n]+unadmitted[ \t\n]*\([ \t\n]*\)"
            r"[ \t\n]*;[ \t\n]*\}"
        ),
        re.compile(
            r"\bnative_payload[ \t\n]*\.[ \t\n]*atom_relocations"
            r"[ \t\n]*\([ \t\n]*\)"
        ),
    ),
)
if scalar_admission_code:
    scalar_admission_prefix_pattern = re.compile(
        r"\Afn[ \t\n]+admit_image[ \t\n]*\([ \t\n]*image[ \t\n]*:"
        r"[ \t\n]*&[ \t\n]*BytecodeImage[ \t\n]*\)[ \t\n]*->"
        r"[ \t\n]*Result[ \t\n]*<[ \t\n]*ScalarScriptDraft[ \t\n]*,"
        r"[ \t\n]*ScalarScriptReadError[ \t\n]*>[ \t\n]*\{"
        r"[ \t\n]*if[ \t\n]+image[ \t\n]*\.[ \t\n]*input_atom_slot_count"
        r"[ \t\n]*\([ \t\n]*\)[ \t\n]*!=[ \t\n]*0[ \t\n]*\{"
        r"[ \t\n]*return[ \t\n]+unadmitted[ \t\n]*\([ \t\n]*\)"
        r"[ \t\n]*;[ \t\n]*\}"
        r"[ \t\n]*if[ \t\n]+![ \t\n]*image[ \t\n]*\.[ \t\n]*atoms"
        r"[ \t\n]*\([ \t\n]*\)[ \t\n]*\.[ \t\n]*is_empty"
        r"[ \t\n]*\([ \t\n]*\)[ \t\n]*\{"
        r"[ \t\n]*return[ \t\n]+unadmitted[ \t\n]*\([ \t\n]*\)"
        r"[ \t\n]*;[ \t\n]*\}"
    )
    if not scalar_admission_prefix_pattern.search(scalar_admission_code):
        fail(
            "scalar-script-admission-empty-boundaries",
            "admit_image must begin with the exact input atom-slot and semantic atom-table rejection guards",
        )

    scalar_function_guard_pattern = re.compile(
        r"\blet[ \t\n]*\[[ \t\n]*function[ \t\n]*\][ \t\n]*="
        r"[ \t\n]*image[ \t\n]*\.[ \t\n]*functions[ \t\n]*\([ \t\n]*\)"
        r"[ \t\n]*else[ \t\n]*\{[ \t\n]*return[ \t\n]+unadmitted"
        r"[ \t\n]*\([ \t\n]*\)[ \t\n]*;[ \t\n]*\}[ \t\n]*;"
    )
    if len(scalar_function_guard_pattern.findall(scalar_admission_code)) != 1:
        fail(
            "scalar-script-admission-empty-boundaries",
            "admit_image must directly bind exactly one decoded function before authenticating its opcode/constant-pool pair",
        )

    scalar_native_guard_pattern = re.compile(
        r"\blet[ \t\n]+native_payload[ \t\n]*=[ \t\n]*envelope"
        r"[ \t\n]*\.[ \t\n]*code[ \t\n]*\([ \t\n]*\)[ \t\n]*;"
        r"[ \t\n]*if[ \t\n]+![ \t\n]*native_payload[ \t\n]*\."
        r"[ \t\n]*atom_relocations[ \t\n]*\([ \t\n]*\)"
        r"[ \t\n]*\.[ \t\n]*is_empty[ \t\n]*\([ \t\n]*\)"
        r"[ \t\n]*\{[ \t\n]*return[ \t\n]+unadmitted"
        r"[ \t\n]*\([ \t\n]*\)[ \t\n]*;[ \t\n]*\}"
    )
    scalar_native_guard_matches = list(
        scalar_native_guard_pattern.finditer(scalar_admission_code)
    )
    if len(scalar_native_guard_matches) != 1:
        fail(
            "scalar-script-admission-empty-boundaries",
            "admit_image must reject atom relocations immediately after binding the decoded native payload",
        )

    scalar_envelope_binding_pattern = re.compile(
        r"\blet[ \t\n]+envelope[ \t\n]*=[ \t\n]*function"
        r"[ \t\n]*\.[ \t\n]*envelope[ \t\n]*\([ \t\n]*\)[ \t\n]*;"
    )
    scalar_envelope_binding_matches = list(
        scalar_envelope_binding_pattern.finditer(scalar_admission_code)
    )
    envelope_binding_is_direct_and_ordered = False
    if len(scalar_envelope_binding_matches) == 1 and len(scalar_native_guard_matches) == 1:
        envelope_match = scalar_envelope_binding_matches[0]
        envelope_prefix = scalar_admission_code[:envelope_match.start()]
        envelope_binding_is_direct_and_ordered = (
            envelope_prefix.count("{") - envelope_prefix.count("}") == 1
            and envelope_match.end() < scalar_native_guard_matches[0].start()
        )
    if not envelope_binding_is_direct_and_ordered:
        fail(
            "scalar-script-admission-empty-boundaries",
            "admit_image must bind the checked envelope once at function scope, directly from the unique decoded function and before the native payload",
        )

    scalar_sequence_binding_pattern = re.compile(
        r"\blet[ \t\n]+Some[ \t\n]*\([ \t\n]*sequence[ \t\n]*\)"
        r"[ \t\n]*=[ \t\n]*decode_scalar_sequence[ \t\n]*\([ \t\n]*native_payload"
        r"[ \t\n]*\.[ \t\n]*as_bytes[ \t\n]*\([ \t\n]*\)[ \t\n]*\)"
        r"[ \t\n]*else[ \t\n]*\{[ \t\n]*return[ \t\n]+unadmitted"
        r"[ \t\n]*\([ \t\n]*\)[ \t\n]*;[ \t\n]*\}[ \t\n]*;"
    )
    scalar_sequence_binding_matches = list(
        scalar_sequence_binding_pattern.finditer(scalar_admission_code)
    )
    if len(scalar_sequence_binding_matches) != 1:
        fail(
            "scalar-script-sequence-admission",
            "admit_image must decode exactly one typed ScalarSequence from the authenticated native payload",
        )

    scalar_sidecar_pattern = re.compile(
        r"\blet[ \t\n]+sidecars_match[ \t\n]*=[ \t\n]*match"
        r"[ \t\n]*\([ \t\n]*&[ \t\n]*sequence[ \t\n]*,"
        r"[ \t\n]*native_payload[ \t\n]*\.[ \t\n]*instructions"
        r"[ \t\n]*\([ \t\n]*\)[ \t\n]*\)[ \t\n]*\{"
    )
    scalar_sidecar_code, sidecar_start, sidecar_close = unique_braced_item(
        scalar_admission_code,
        scalar_sidecar_pattern,
        "scalar-script-sequence-sidecars",
        "typed scalar sequence/instruction-sidecar match",
    )
    if scalar_sidecar_code:
        expected_sidecar_source = """
            let sidecars_match = match (&sequence, native_payload.instructions()) {
                (ScalarSequence::Plain { width, .. }, [push, set_completion, return_value]) => {
                    push.offset() == 0
                        && push.opcode().raw() == native_payload.as_bytes()[0]
                        && set_completion.offset() == *width
                        && set_completion.opcode().raw() == OP_SET_LOC0
                        && return_value.offset() == *width + 1
                        && return_value.opcode().raw() == OP_RETURN
                }
                (
                    ScalarSequence::NegatedBigInt { width, .. },
                    [push, negate, set_completion, return_value],
                ) => {
                    push.offset() == 0
                        && push.opcode().raw() == native_payload.as_bytes()[0]
                        && negate.offset() == *width
                        && negate.opcode().raw() == OP_NEG
                        && set_completion.offset() == *width + 1
                        && set_completion.opcode().raw() == OP_SET_LOC0
                        && return_value.offset() == *width + 2
                        && return_value.opcode().raw() == OP_RETURN
                }
                _ => false,
            }
        """
        sidecar_guard_pattern = re.compile(
            r"[ \t\n]*;[ \t\n]*if[ \t\n]+![ \t\n]*sidecars_match"
            r"[ \t\n]*\{[ \t\n]*return[ \t\n]+Err"
            r"[ \t\n]*\([ \t\n]*ScalarScriptReadError[ \t\n]*::"
            r"[ \t\n]*Internal[ \t\n]*\(.*?\)[ \t\n]*\)"
            r"[ \t\n]*;[ \t\n]*\}",
            re.DOTALL,
        )
        sidecar_is_direct_and_guarded = (
            len(scalar_sequence_binding_matches) == 1
            and scalar_sequence_binding_matches[0].end() < sidecar_start
            and scalar_admission_code[:sidecar_start].count("{")
            - scalar_admission_code[:sidecar_start].count("}")
            == 1
            and sidecar_guard_pattern.match(scalar_admission_code, sidecar_close)
            is not None
        )
        if (
            " ".join(scalar_sidecar_code.split())
            != " ".join(rust_code_only(expected_sidecar_source).split())
            or not sidecar_is_direct_and_guarded
        ):
            fail(
                "scalar-script-sequence-sidecars",
                "plain scalars must authenticate exactly three sidecars, while a BigInt-only single neg must authenticate exactly push, OP_NEG, set_loc0, and return at the reviewed offsets",
            )

    scalar_pairing_pattern = re.compile(
        r"\blet[ \t\n]+draft[ \t\n]*=[ \t\n]*match[ \t\n]*\("
        r"[ \t\n]*sequence[ \t\n]*,[ \t\n]*function[ \t\n]*\."
        r"[ \t\n]*constants[ \t\n]*\([ \t\n]*\)[ \t\n]*\)"
        r"[ \t\n]*\{"
    )
    scalar_pairing_code, pairing_start, pairing_close = unique_braced_item(
        scalar_admission_code,
        scalar_pairing_pattern,
        "scalar-script-constant-pairing",
        "draft-producing scalar push/constant-pool match",
    )

    if scalar_pairing_code:
        expected_pairing_source = """
            let draft = match (sequence, function.constants()) {
                (
                    ScalarSequence::Plain {
                        push: ScalarPush::Direct(draft),
                        ..
                    },
                    [],
                ) => draft,
                (
                    ScalarSequence::Plain {
                        push: ScalarPush::Direct(_),
                        ..
                    },
                    _,
                ) => {
                    return unadmitted("direct scalar opcode carries a function constant");
                }
                (
                    ScalarSequence::Plain {
                        push: ScalarPush::Constant(0),
                        ..
                    },
                    [constant],
                ) => match constant.as_wire() {
                    Ok(WireValue::Float64Bits(bits)) => ScalarScriptDraft::Float64Bits(*bits),
                    Ok(WireValue::BigInt(bytes)) => {
                        ScalarScriptDraft::BigIntBytes(copy_bigint_bytes(bytes)?)
                    }
                    Ok(_) => return unadmitted("scalar constant is not a Float64 or BigInt value"),
                    Err(_) => return unadmitted("scalar constant is not a data value"),
                },
                (
                    ScalarSequence::Plain {
                        push: ScalarPush::Constant(_),
                        ..
                    },
                    [_],
                ) => {
                    return unadmitted("scalar constant opcode does not reference index zero");
                }
                (
                    ScalarSequence::Plain {
                        push: ScalarPush::Constant(_),
                        ..
                    },
                    _,
                ) => {
                    return unadmitted("scalar constant opcode requires exactly one function constant");
                }
                (
                    ScalarSequence::NegatedBigInt {
                        push: BigIntPush::DirectI32(value),
                        ..
                    },
                    [],
                ) => ScalarScriptDraft::NegatedBigIntI32(value),
                (
                    ScalarSequence::NegatedBigInt {
                        push: BigIntPush::DirectI32(_),
                        ..
                    },
                    _,
                ) => return unadmitted("direct negated BigInt opcode carries a function constant"),
                (
                    ScalarSequence::NegatedBigInt {
                        push: BigIntPush::Constant(0),
                        ..
                    },
                    [constant],
                ) => match constant.as_wire() {
                    Ok(WireValue::BigInt(bytes)) => {
                        ScalarScriptDraft::NegatedBigIntBytes(copy_bigint_bytes(bytes)?)
                    }
                    Ok(_) => return unadmitted("negated scalar constant is not a BigInt value"),
                    Err(_) => return unadmitted("negated scalar constant is not a data value"),
                },
                (
                    ScalarSequence::NegatedBigInt {
                        push: BigIntPush::Constant(_),
                        ..
                    },
                    [_],
                ) => return unadmitted("negated BigInt opcode does not reference index zero"),
                (
                    ScalarSequence::NegatedBigInt {
                        push: BigIntPush::Constant(_),
                        ..
                    },
                    _,
                ) => {
                    return unadmitted("negated BigInt opcode requires exactly one function constant");
                }
            }
        """
        pairing_is_direct_and_final = (
            len(scalar_sequence_binding_matches) == 1
            and scalar_sequence_binding_matches[0].end() < pairing_start
            and scalar_admission_code[:pairing_start].count("{")
            - scalar_admission_code[:pairing_start].count("}")
            == 1
            and re.fullmatch(
                r"[ \t\n]*;[ \t\n]*Ok[ \t\n]*\([ \t\n]*draft[ \t\n]*\)"
                r"[ \t\n]*\}[ \t\n]*",
                scalar_admission_code[pairing_close:],
            ) is not None
        )
        if (
            " ".join(scalar_pairing_code.split())
            != " ".join(rust_code_only(expected_pairing_source).split())
            or not pairing_is_direct_and_final
        ):
            fail(
                "scalar-script-constant-pairing",
                "admission must atomically pair plain scalars or BigInt-only single-neg sequences with the exact reviewed empty or index-zero/one-entry pool shape",
            )

    scalar_production_code = scalar_script_code.split("#[cfg(test)]", 1)[0]
    if re.findall(
        r"\bWireValue[ \t\n]*::[ \t\n]*([A-Za-z_][A-Za-z0-9_]*)",
        scalar_production_code,
    ) != ["Float64Bits", "BigInt", "BigInt"]:
        fail(
            "scalar-script-constant-pairing",
            "the scalar-script path may name only the three reviewed Float64/BigInt pool variants",
        )

    reviewed_binding_counts = {
        "function": 0,
        "envelope": 0,
        "native_payload": 0,
        "sequence": 0,
        "sidecars_match": 0,
        "draft": 0,
    }
    for binding_match in re.finditer(
        r"\blet\b(?P<pattern>[^=;]+)=", scalar_admission_code
    ):
        binding_pattern = binding_match.group("pattern")
        for binding_name in reviewed_binding_counts:
            reviewed_binding_counts[binding_name] += len(
                re.findall(rf"\b{binding_name}\b", binding_pattern)
            )
    if reviewed_binding_counts != {
        "function": 1,
        "envelope": 1,
        "native_payload": 1,
        "sequence": 1,
        "sidecars_match": 1,
        "draft": 1,
    }:
        fail(
            "scalar-script-admission-empty-boundaries",
            "admit_image must bind function, envelope, native_payload, sequence, sidecars_match, and draft exactly once; "
            f"found {reviewed_binding_counts}",
        )

    if re.search(r"\blet[ \t\n]+(?:mut[ \t\n]+)?image\b", scalar_admission_code):
        fail(
            "scalar-script-admission-empty-boundaries",
            "admit_image must not shadow its authenticated image receiver",
        )

    allowed_return_pattern = re.compile(
        r"\breturn[ \t\n]+(?:unadmitted|Err)[ \t\n]*\("
    )
    for return_match in re.finditer(r"\breturn\b", scalar_admission_code):
        if allowed_return_pattern.match(scalar_admission_code, return_match.start()) is None:
            fail(
                "scalar-script-admission-empty-boundaries",
                "admit_image may return early only through the reviewed unadmitted or internal-error paths",
            )

    success_matches = list(
        re.finditer(
            r"\bOk[ \t\n]*\([ \t\n]*draft[ \t\n]*\)",
            scalar_admission_code,
        )
    )
    if len(success_matches) != 1 or re.search(
        r"\bOk[ \t\n]*\([ \t\n]*draft[ \t\n]*\)[ \t\n]*\}[ \t\n]*\Z",
        scalar_admission_code,
    ) is None:
        fail(
            "scalar-script-admission-empty-boundaries",
            "admit_image must have exactly one successful exit: the final Ok(draft)",
        )

    for description, statement_pattern, call_pattern in scalar_admission_empty_boundaries:
        statement_matches = list(statement_pattern.finditer(scalar_admission_code))
        call_matches = list(call_pattern.finditer(scalar_script_code))
        direct_statement = False
        if len(statement_matches) == 1:
            statement_offset = statement_matches[0].start()
            prefix = scalar_admission_code[:statement_offset]
            direct_statement = prefix.count("{") - prefix.count("}") == 1
        if len(statement_matches) != 1 or len(call_matches) != 1 or not direct_statement:
            fail(
                "scalar-script-admission-empty-boundaries",
                "admit_image must directly and exactly once reject a non-empty "
                f"{description}",
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
    if len(consumer_facade_imports) != 1:
        fail(
            "binary-object-consumer-import",
            f"{consumer_relative} must contain exactly one reviewed scalar facade import",
        )
    else:
        consumer_import_items = [
            item.strip()
            for item in consumer_facade_imports[0].group("body").split(",")
            if item.strip()
        ]
        if (
            len(consumer_import_items) != len(expected_scalar_facade_names)
            or set(consumer_import_items) != expected_scalar_facade_names
        ):
            fail(
                "binary-object-consumer-import",
                f"{consumer_relative} may import only the reviewed scalar facade; "
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

    lowered_scalar_pattern = re.compile(
        r"(?m)^[ \t]*enum[ \t]+LoweredScalar[ \t\n]*\{"
        r"[ \t\n]*Direct[ \t\n]*\([ \t\n]*Instruction[ \t\n]*\)[ \t\n]*,"
        r"[ \t\n]*Constant[ \t\n]*\([ \t\n]*UnlinkedConstant[ \t\n]*\)[ \t\n]*,"
        r"[ \t\n]*NegatedBigInt[ \t\n]*\([ \t\n]*UnlinkedConstant[ \t\n]*\)"
        r"[ \t\n]*,?[ \t\n]*\}"
    )
    if len(lowered_scalar_pattern.findall(consumer_code)) != 1:
        fail(
            "binary-object-consumer-scalar-mapping",
            "LoweredScalar must remain one private Direct push, primitive Constant, or NegatedBigInt constant discriminator",
        )

    if len(
        re.findall(r"\bInstruction[ \t\n]*::[ \t\n]*Neg\b", consumer_code)
    ) != 1:
        fail(
            "binary-object-consumer-bigint-negation",
            f"{consumer_relative} must contain exactly one execution-time Instruction::Neg",
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
            let draft = decode_trusted_scalar_script(bytes).map_err(map_read_error)?;
            let (instructions, constants) = match lower_scalar_draft(draft)? {
                LoweredScalar::Direct(push) => (
                    vec![push, Instruction::SetLocal(0), Instruction::Return],
                    Vec::new(),
                ),
                LoweredScalar::Constant(constant) => (
                    vec![
                        Instruction::PushConst(0),
                        Instruction::SetLocal(0),
                        Instruction::Return,
                    ],
                    vec![constant],
                ),
                LoweredScalar::NegatedBigInt(constant) => (
                    vec![
                        Instruction::PushConst(0),
                        Instruction::Neg,
                        Instruction::SetLocal(0),
                        Instruction::Return,
                    ],
                    vec![constant],
                ),
            };
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
            f"{consumer_relative} must uniquely assemble the reviewed Direct, Constant, or NegatedBigInt three/four-instruction shape before entering the ordinary verifier",
        )

    scalar_lowering_pattern = re.compile(
        r"\bfn[ \t\n]+lower_scalar_draft[ \t\n]*\([^{};]*\)"
        r"[ \t\n]*->[^{;]+\{",
        re.DOTALL,
    )
    scalar_lowering_code, _, _ = unique_braced_item(
        consumer_code,
        scalar_lowering_pattern,
        "binary-object-consumer-scalar-mapping",
        "lower_scalar_draft function",
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
    bigint_negation_pattern = re.compile(
        r"\bfn[ \t\n]+lower_negated_bigint[ \t\n]*\("
        r"[ \t\n]*value[ \t\n]*:[ \t\n]*JsBigInt[ \t\n]*,?"
        r"[ \t\n]*\)[ \t\n]*->[^{;]+\{",
        re.DOTALL,
    )
    bigint_negation_code, _, _ = unique_braced_item(
        consumer_code,
        bigint_negation_pattern,
        "binary-object-consumer-bigint-negation",
        "lower_negated_bigint function",
    )
    scalar_constant_pattern = re.compile(
        r"\bfn[ \t\n]+lower_scalar_constant[ \t\n]*\([^{};]*\)"
        r"[ \t\n]*->[^{;]+\{",
        re.DOTALL,
    )
    scalar_constant_code, _, _ = unique_braced_item(
        consumer_code,
        scalar_constant_pattern,
        "binary-object-consumer-scalar-mapping",
        "lower_scalar_constant function",
    )
    expected_scalar_lowering_source = """
        fn lower_scalar_draft(draft: ScalarScriptDraft) -> Result<LoweredScalar, RuntimeError> {
            match draft {
                ScalarScriptDraft::Undefined => Ok(LoweredScalar::Direct(Instruction::Undefined)),
                ScalarScriptDraft::Null => Ok(LoweredScalar::Direct(Instruction::Null)),
                ScalarScriptDraft::Bool(false) => Ok(LoweredScalar::Direct(Instruction::PushFalse)),
                ScalarScriptDraft::Bool(true) => Ok(LoweredScalar::Direct(Instruction::PushTrue)),
                ScalarScriptDraft::Int(value) => Ok(LoweredScalar::Direct(Instruction::PushI32(value))),
                ScalarScriptDraft::Float64Bits(bits) => {
                    lower_scalar_constant(Value::Float(f64::from_bits(bits))).map(LoweredScalar::Constant)
                }
                ScalarScriptDraft::BigIntI32(value) => {
                    lower_scalar_constant(Value::BigInt(JsBigInt::from(value))).map(LoweredScalar::Constant)
                }
                ScalarScriptDraft::BigIntBytes(bytes) => {
                    lower_bigint_constant(&bytes).map(LoweredScalar::Constant)
                }
                ScalarScriptDraft::NegatedBigIntI32(value) => lower_negated_bigint(JsBigInt::from(value)),
                ScalarScriptDraft::NegatedBigIntBytes(bytes) => {
                    lower_negated_bigint(decode_bigint_constant(&bytes)?)
                }
                ScalarScriptDraft::EmptyString => {
                    lower_scalar_constant(Value::String(JsString::from_static("")))
                        .map(LoweredScalar::Constant)
                }
            }
        }
    """
    expected_bigint_lowering_source = """
        fn lower_bigint_constant(bytes: &[u8]) -> Result<UnlinkedConstant, RuntimeError> {
            lower_scalar_constant(Value::BigInt(decode_bigint_constant(bytes)?))
        }
    """
    expected_bigint_decoder_source = """
        fn decode_bigint_constant(bytes: &[u8]) -> Result<JsBigInt, RuntimeError> {
            let (value, consumed) =
                JsBigInt::decode_bc5_signed_le(bytes, bytes.len(), bytes.len(), true)
            .map_err(|error| {
                RuntimeError::Engine(Error::internal(format!(
                    "trusted scalar draft contained invalid canonical BigInt bytes: {error:?}"
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
    expected_bigint_negation_source = """
        fn lower_negated_bigint(value: JsBigInt) -> Result<LoweredScalar, RuntimeError> {
            lower_scalar_constant(Value::BigInt(value)).map(LoweredScalar::NegatedBigInt)
        }
    """
    expected_scalar_constant_source = """
        fn lower_scalar_constant(value: Value) -> Result<UnlinkedConstant, RuntimeError> {
            UnlinkedConstant::primitive(value).map_err(|error| {
                RuntimeError::Engine(Error::internal(format!(
                    "trusted scalar draft produced an invalid primitive constant: {error}"
                )))
            })
        }
    """
    if (
        " ".join(scalar_lowering_code.split())
        != " ".join(rust_code_only(expected_scalar_lowering_source).split())
        or re.findall(
            r"\bScalarScriptDraft[ \t\n]*::[ \t\n]*([A-Za-z_][A-Za-z0-9_]*)",
            consumer_code,
        ) != [
            "Undefined",
            "Null",
            "Bool",
            "Bool",
            "Int",
            "Float64Bits",
            "BigIntI32",
            "BigIntBytes",
            "NegatedBigIntI32",
            "NegatedBigIntBytes",
            "EmptyString",
        ]
    ):
        fail(
            "binary-object-consumer-scalar-mapping",
            f"{consumer_relative} must retain the entire reviewed direct-scalar, bit-exact Float64, plain BigInt, and strongly typed negated-BigInt lowering contract",
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
    if (
        " ".join(bigint_negation_code.split())
        != " ".join(rust_code_only(expected_bigint_negation_source).split())
        or len(re.findall(r"\blower_negated_bigint\b", consumer_code)) != 3
        or len(
            re.findall(
                r"\bLoweredScalar[ \t\n]*::[ \t\n]*NegatedBigInt\b",
                consumer_code,
            )
        )
        != 2
    ):
        fail(
            "binary-object-consumer-bigint-negation",
            f"{consumer_relative} must preserve BigInt unary negation as exactly one execution-time Instruction::Neg after a primitive constant push",
        )
    for match in re.finditer(r"\b(?:r#)?number[ \t\n]*\(", consumer_code):
        fail(
            "binary-object-consumer-float64",
            f"{consumer_relative} must not normalize an authenticated Float64 tag through Value::number or an alias; found "
            + location(consumer_relative, consumer_source, match.start()),
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
            "binary-object-consumer-verifier-bypass",
            re.compile(r"\bpublish_verified_unlinked_function\b"),
            "publish_verified_unlinked_function",
        ),
        (
            "binary-object-consumer-alternate-entrypoint",
            re.compile(
                r"\b(?:(?:self|runtime)[ \t\n]*\.[ \t\n]*|Runtime[ \t\n]*::[ \t\n]*)"
                r"(?!(?:publish_unlinked_function)\b)"
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
            "binary-object-consumer-atom-string",
            re.compile(r"\batom_string\b"),
            "the atom_string identifier, including through an alias",
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

src_root = root / "src"
if src_root.is_symlink() or not src_root.is_dir():
    fail("missing-source", "src must be a regular directory")
    production_sources: list[Path] = []
else:
    production_sources = sorted(src_root.rglob("*.rs"))

facade_name_pattern = re.compile(
    r"\b(?:ScalarScriptDraft|ScalarScriptReadError|decode_trusted_scalar_script)\b"
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
                "scalar-script-consumer-set",
                "only binary_object_publish.rs may name the scalar-script facade; found "
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
    len(null_name_predicate.findall(image_model_code)) != 1
    or len(eval_name_predicate.findall(image_model_code)) != 1
):
    fail(
        "scalar-script-atom-predicate",
        "the model must expose only the reviewed null-local and pinned-<eval> boolean predicates",
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
    "$fixture/src/runtime/binary_object/graph"
printf '%s\n' 'pub mod runtime;' > "$fixture/src/lib.rs"
printf '%s\n' 'mod binary_object;' > "$fixture/src/runtime.rs"
printf '%s\n' \
    'mod atoms;' \
    'mod code;' \
    'mod function_envelope;' \
    'mod bytecode_image;' \
    'mod graph;' \
    'mod pinned_atoms;' \
    'mod pinned_opcodes;' \
    'mod read_cursor;' \
    'mod scalar_script;' \
    'mod wire;' \
    'pub(super) use scalar_script::{ScalarScriptDraft, ScalarScriptReadError, decode_trusted_scalar_script};' \
    > "$fixture/src/runtime/binary_object/mod.rs"
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
    > "$fixture/src/runtime/binary_object/bytecode_image/mod.rs"
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
    'use super::binary_object::{ScalarScriptDraft, decode_trusted_scalar_script};'
expect_rejected alternate-binary-object-path binary-object-consumer-set \
    src/runtime/other.rs \
    '#[path = "binary_object/mod.rs"] mod alternate_archive;'
expect_rejected second-scalar-facade-consumer scalar-script-consumer-set \
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
expect_rejected consumer-verifier-bypass binary-object-consumer-verifier-bypass \
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
expect_rewrite_rejected consumer-lowered-scalar-vector binary-object-consumer-scalar-mapping \
    src/runtime/binary_object_publish.rs \
    '    NegatedBigInt(UnlinkedConstant),' \
    '    NegatedBigInt(Vec<Instruction>),'
expect_rewrite_rejected consumer-float-normalization binary-object-consumer-float64 \
    src/runtime/binary_object_publish.rs \
    'lower_scalar_constant(Value::Float(f64::from_bits(bits)))' \
    'lower_scalar_constant(Value::number(f64::from_bits(bits)))'
expect_rewrite_rejected consumer-bigint-dead-path-coercion binary-object-consumer-scalar-mapping \
    src/runtime/binary_object_publish.rs \
    $'        ScalarScriptDraft::BigIntBytes(bytes) => {\n            lower_bigint_constant(&bytes).map(LoweredScalar::Constant)\n        }' \
    $'        ScalarScriptDraft::BigIntBytes(bytes) => {\n            if false { return lower_bigint_constant(&bytes).map(LoweredScalar::Constant); }\n            Ok(LoweredScalar::Direct(Instruction::PushI32(i32::from(bytes.first().copied().unwrap_or(0)))))\n        }'
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
expect_rewrite_rejected consumer-bigint-negation-omitted binary-object-consumer-bigint-negation \
    src/runtime/binary_object_publish.rs \
    'Instruction::Neg,' \
    'Instruction::Nop,'
expect_rejected consumer-bigint-eager-negation binary-object-consumer-bigint-eager-negation \
    src/runtime/binary_object_publish.rs \
    'fn eager_negation(value: JsBigInt) { let _ = std::ops::Neg::neg(value); }'
expect_rewrite_rejected consumer-skips-safe-publication binary-object-consumer-publication \
    src/runtime/binary_object_publish.rs \
    '        self.publish_unlinked_function(realm, function)' \
    '        self.compile_in_realm(realm, source)'
expect_rejected root-public-module root-module-visibility \
    src/runtime/binary_object/mod.rs \
    'pub(in crate::runtime) mod leaked;'
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
    'pub(super) use scalar_script::{ScalarScriptDraft, ScalarScriptReadError, decode_trusted_scalar_script};' \
    'pub(super) use scalar_script::{BytecodeImage, ScalarScriptDraft, ScalarScriptReadError, decode_trusted_scalar_script};'
expect_rewrite_rejected scalar-facade-wider-visibility scalar-script-facade-shape \
    src/runtime/binary_object/mod.rs \
    'pub(super) use scalar_script::{ScalarScriptDraft, ScalarScriptReadError, decode_trusted_scalar_script};' \
    'pub(crate) use scalar_script::{ScalarScriptDraft, ScalarScriptReadError, decode_trusted_scalar_script};'
expect_rewrite_rejected scalar-draft-raw-c-forgery scalar-script-draft-shape \
    src/runtime/binary_object/scalar_script.rs \
    $'pub(in crate::runtime) enum ScalarScriptDraft {\n    Undefined,\n    Null,\n    Bool(bool),\n    Int(i32),\n    Float64Bits(u64),\n    BigIntI32(i32),\n    BigIntBytes(Box<[u8]>),\n    NegatedBigIntI32(i32),\n    NegatedBigIntBytes(Box<[u8]>),\n    EmptyString,\n}' \
    $'enum FloatDraft { Float(f64) }\nconst _FLOAT_DRAFT_FORGERY: &CStr = cr#""\n#[derive(Clone, Debug, Eq, PartialEq)]\npub(in crate::runtime) enum ScalarScriptDraft {\n    Undefined,\n    Null,\n    Bool(bool),\n    Int(i32),\n    Float64Bits(u64),\n    BigIntI32(i32),\n    BigIntBytes(Box<[u8]>),\n    NegatedBigIntI32(i32),\n    NegatedBigIntBytes(Box<[u8]>),\n    EmptyString,\n}\n""#;'
expect_rewrite_rejected scalar-draft-copy-regression scalar-script-draft-shape \
    src/runtime/binary_object/scalar_script.rs \
    $'#[derive(Clone, Debug, Eq, PartialEq)]\npub(in crate::runtime) enum ScalarScriptDraft' \
    $'#[derive(Clone, Copy, Debug, Eq, PartialEq)]\npub(in crate::runtime) enum ScalarScriptDraft'
expect_rewrite_rejected scalar-push-copy-regression scalar-script-push-shape \
    src/runtime/binary_object/scalar_script.rs \
    $'#[derive(Clone, Debug, Eq, PartialEq)]\nenum ScalarPush' \
    $'#[derive(Clone, Copy, Debug, Eq, PartialEq)]\nenum ScalarPush'
expect_rejected scalar-opcode-set-widening scalar-script-opcode-set \
    src/runtime/binary_object/scalar_script.rs \
    'const OP_PUSH_THIS: u8 = 0x08;'
expect_rewrite_rejected scalar-opcode-duplicate scalar-script-opcode-set \
    src/runtime/binary_object/scalar_script.rs \
    'const OP_NULL: u8 = 0x07;' \
    $'    const OP_NULL: u8 = 0xfe + 1;\nconst OP_NULL: u8 = 0x07;'
expect_rewrite_rejected scalar-const8-index-forgery scalar-script-push-decoder \
    src/runtime/binary_object/scalar_script.rs \
    'ScalarPush::Constant(u32::from(*index))' \
    'ScalarPush::Constant(0)'
expect_rewrite_rejected scalar-fclosure8-substitution scalar-script-push-decoder \
    src/runtime/binary_object/scalar_script.rs \
    '[OP_PUSH_CONST8, index, OP_SET_LOC0, OP_RETURN]' \
    '[0xbe, index, OP_SET_LOC0, OP_RETURN]'
expect_rewrite_rejected scalar-neg-wildcard scalar-script-sequence-decoder \
    src/runtime/binary_object/scalar_script.rs \
    '        _ => decode_scalar_push(bytes).map(|(push, width)| ScalarSequence::Plain { push, width }),' \
    '        _ if bytes.contains(&OP_NEG) => Some(ScalarSequence::NegatedBigInt { push: BigIntPush::Constant(0), width: 2 }),'
expect_rewrite_rejected scalar-double-neg scalar-script-sequence-decoder \
    src/runtime/binary_object/scalar_script.rs \
    '[OP_PUSH_CONST8, index, OP_NEG, OP_SET_LOC0, OP_RETURN]' \
    '[OP_PUSH_CONST8, index, OP_NEG, OP_NEG, OP_SET_LOC0, OP_RETURN]'
expect_rewrite_rejected scalar-int-neg scalar-script-sequence-decoder \
    src/runtime/binary_object/scalar_script.rs \
    $'            OP_PUSH_BIGINT_I32,\n            byte_0,\n            byte_1,\n            byte_2,\n            byte_3,\n            OP_NEG,' \
    $'            OP_PUSH_I32,\n            byte_0,\n            byte_1,\n            byte_2,\n            byte_3,\n            OP_NEG,'
expect_rewrite_rejected scalar-float-neg scalar-script-constant-pairing \
    src/runtime/binary_object/scalar_script.rs \
    $'        ) => match constant.as_wire() {\n            Ok(WireValue::BigInt(bytes)) => {\n                ScalarScriptDraft::NegatedBigIntBytes' \
    $'        ) => match constant.as_wire() {\n            Ok(WireValue::Float64Bits(bytes)) => {\n                ScalarScriptDraft::NegatedBigIntBytes'
expect_rewrite_rejected scalar-constant-index-widening scalar-script-constant-pairing \
    src/runtime/binary_object/scalar_script.rs \
    '                push: ScalarPush::Constant(0),' \
    '                push: ScalarPush::Constant(_),'
expect_rewrite_rejected scalar-constant-extra-pool scalar-script-constant-pairing \
    src/runtime/binary_object/scalar_script.rs \
    $'            [constant],\n        ) => match constant.as_wire() {\n            Ok(WireValue::Float64Bits(bits))' \
    $'            [constant, ..],\n        ) => match constant.as_wire() {\n            Ok(WireValue::Float64Bits(bits))'
expect_rewrite_rejected scalar-constant-wrong-type scalar-script-constant-pairing \
    src/runtime/binary_object/scalar_script.rs \
    'Ok(WireValue::Float64Bits(bits))' \
    'Ok(WireValue::Int32(bits))'
expect_rewrite_rejected scalar-constant-wrong-type-comment-forgery scalar-script-constant-pairing \
    src/runtime/binary_object/scalar_script.rs \
    'Ok(WireValue::Float64Bits(bits))' \
    'Ok(WireValue::Int32(bits)) /* WireValue::Float64Bits */'
expect_rewrite_rejected scalar-constant-string-opening scalar-script-constant-pairing \
    src/runtime/binary_object/scalar_script.rs \
    '            Ok(_) => return unadmitted("scalar constant is not a Float64 or BigInt value"),' \
    $'            Ok(WireValue::String(_)) => ScalarScriptDraft::EmptyString,\n            Ok(_) => return unadmitted("scalar constant is not a Float64 or BigInt value"),'
expect_rewrite_rejected scalar-constant-bool-opening scalar-script-constant-pairing \
    src/runtime/binary_object/scalar_script.rs \
    '            Ok(_) => return unadmitted("scalar constant is not a Float64 or BigInt value"),' \
    $'            Ok(WireValue::Bool(value)) => ScalarScriptDraft::Bool(*value),\n            Ok(_) => return unadmitted("scalar constant is not a Float64 or BigInt value"),'
expect_rewrite_rejected scalar-constant-wildcard-primitive scalar-script-constant-pairing \
    src/runtime/binary_object/scalar_script.rs \
    '            Ok(_) => return unadmitted("scalar constant is not a Float64 or BigInt value"),' \
    '            Ok(_) => ScalarScriptDraft::BigIntBytes(Box::default()),'
expect_rewrite_rejected scalar-bigint-truncated-copy scalar-script-constant-pairing \
    src/runtime/binary_object/scalar_script.rs \
    'ScalarScriptDraft::BigIntBytes(copy_bigint_bytes(bytes)?)' \
    'ScalarScriptDraft::BigIntBytes(copy_bigint_bytes(&bytes[..1])?)'
expect_rewrite_rejected scalar-bigint-infallible-copy scalar-script-bigint-copy \
    src/runtime/binary_object/scalar_script.rs \
    'copy.try_reserve_exact(bytes.len())' \
    'copy.reserve(bytes.len())'
expect_rewrite_rejected scalar-direct-with-pool scalar-script-constant-pairing \
    src/runtime/binary_object/scalar_script.rs \
    $'                ..\n            },\n            [],\n        ) => draft,' \
    $'                ..\n            },\n            [_],\n        ) => draft,'
expect_rewrite_rejected scalar-constant-pairing-bypass scalar-script-constant-pairing \
    src/runtime/binary_object/scalar_script.rs \
    '    let draft = match (sequence, function.constants()) {' \
    $'    let draft = ScalarScriptDraft::Float64Bits(0);\n    let _reviewed_pair = match (sequence, function.constants()) {'
expect_rejected scalar-float-evidence-alias scalar-script-constant-pairing \
    src/runtime/binary_object/scalar_script.rs \
    'use WireValue::Float64Bits as AdmittedFloat;'
expect_rewrite_rejected scalar-input-atom-slot-widening scalar-script-admission-empty-boundaries \
    src/runtime/binary_object/scalar_script.rs \
    '    if image.input_atom_slot_count() != 0 {' \
    '    if image.input_atom_slot_count() != 1 {'
expect_rewrite_rejected scalar-input-atom-slot-comment-forgery scalar-script-admission-empty-boundaries \
    src/runtime/binary_object/scalar_script.rs \
    '    if image.input_atom_slot_count() != 0 {' \
    '    if false { /* image.input_atom_slot_count() != 0 */'
expect_rewrite_rejected scalar-admission-early-success scalar-script-admission-empty-boundaries \
    src/runtime/binary_object/scalar_script.rs \
    '    if image.input_atom_slot_count() != 0 {' \
    '    return Ok(ScalarScriptDraft::EmptyString); if image.input_atom_slot_count() != 0 {'
expect_rewrite_rejected scalar-admission-image-shadow scalar-script-admission-empty-boundaries \
    src/runtime/binary_object/scalar_script.rs \
    '    if image.input_atom_slot_count() != 0 {' \
    '    let image = image; if image.input_atom_slot_count() != 0 {'
expect_rewrite_rejected scalar-admission-envelope-shadow scalar-script-admission-empty-boundaries \
    src/runtime/binary_object/scalar_script.rs \
    '    let native_payload = envelope.code();' \
    '    let envelope = function.envelope(); let native_payload = envelope.code();'
expect_rewrite_rejected scalar-admission-dead-envelope scalar-script-admission-empty-boundaries \
    src/runtime/binary_object/scalar_script.rs \
    '    let envelope = function.envelope();' \
    '    if false { let envelope = function.envelope(); }'
expect_rewrite_rejected scalar-admission-native-payload-shadow scalar-script-admission-empty-boundaries \
    src/runtime/binary_object/scalar_script.rs \
    '    let draft = match (sequence, function.constants()) {' \
    '    let native_payload = envelope.code(); let draft = match (sequence, function.constants()) {'
expect_rewrite_rejected scalar-error-missing-unadmitted scalar-script-error-shape \
    src/runtime/binary_object/scalar_script.rs \
    '    Unadmitted(String),' \
    '    Rejected(String),'
expect_rejected scalar-extra-visible-item scalar-script-visible-item-set \
    src/runtime/binary_object/scalar_script.rs \
    'pub(in crate::runtime) fn leak_image() {}'
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
expect_rewrite_rejected pinned-eval-identity-drift scalar-script-atom-predicate \
    src/runtime/binary_object/bytecode_image/model.rs \
    'const PINNED_EVAL_ATOM_RAW: u32 = 84;' \
    'const PINNED_EVAL_ATOM_RAW: u32 = 85;'
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
