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

        raw = re.match(r'(?:br|rb|r)(?P<hashes>#{0,255})"', source[index:])
        if raw is not None:
            start = index
            hashes = raw.group("hashes")
            index += raw.end()
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
for match in public_use_pattern.finditer(binary_root_code):
    fail(
        "root-reexport",
        "binary_object root must not re-export codec internals; found "
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
    source = path.read_text(encoding="utf-8")
    code = rust_code_only(source)
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
graph_decode_source = read_source(graph_decode_relative)
graph_decode_code = rust_code_only(graph_decode_source)
image_decode_source = read_source(image_decode_relative)
image_decode_code = rust_code_only(image_decode_source)
sab_transport_source = read_source(sab_transport_relative)
sab_transport_code = rust_code_only(sab_transport_source)
image_model_source = read_source(image_model_relative)
image_model_code = rust_code_only(image_model_source)

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
    source = path.read_text(encoding="utf-8")
    code = rust_code_only(source)
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
    code = rust_code_only(path.read_text(encoding="utf-8"))
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
        code = rust_code_only(path.read_text(encoding="utf-8"))
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
        code = rust_code_only(path.read_text(encoding="utf-8"))
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
    code = rust_code_only(path.read_text(encoding="utf-8"))
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
    code = rust_code_only(path.read_text(encoding="utf-8"))
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
    code = rust_code_only(path.read_text(encoding="utf-8"))
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
    code = rust_code_only(path.read_text(encoding="utf-8"))
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
    source = path.read_text(encoding="utf-8")
    code = rust_code_only(source)
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
            "forbidden-shared-memory-dependency",
            re.compile(r"\bcrate[ \t\n]*::[ \t\n]*(?:r#)?shared_memory\b"),
            "crate::shared_memory",
        ),
        (
            "forbidden-parent-dependency",
            re.compile(
                r"\b(?:super[ \t\n]*::[ \t\n]*)+(?:r#)?(?:vm|compiler)\b"
            ),
            "a parent-relative VM/compiler path",
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
        if re.search(r"(?:^|[,{}])[ \t\n]*(?:r#)?shared_memory\b", grouped.group("body")):
            fail(
                "forbidden-shared-memory-dependency",
                "binary_object production sources must not import shared_memory through a parent-relative grouped use; found "
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
    'mod wire;' \
    > "$fixture/src/runtime/binary_object/mod.rs"
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

scan_root "$fixture" || die "binary-object boundary rejected its clean self-test fixture"

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
expect_rejected root-public-module root-module-visibility \
    src/runtime/binary_object/mod.rs \
    'pub(in crate::runtime) mod leaked;'
expect_rejected root-reexport root-reexport \
    src/runtime/binary_object/mod.rs \
    'pub(in crate::runtime) use bytecode_image::*;'
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
