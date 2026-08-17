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
mkdir -p "$fixture/src/runtime/binary_object/bytecode_image"
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
    'mod wire;' \
    > "$fixture/src/runtime/binary_object/mod.rs"
printf '%s\n' \
    'fn retained_from_raw(raw: u32) {' \
    '    let _ = PinnedAtomId::from_raw(raw);' \
    '}' \
    > "$fixture/src/runtime/binary_object/atoms.rs"
printf '%s\n' \
    'mod atoms;' \
    'mod budget;' \
    'mod decode;' \
    'mod encode;' \
    'mod model;' \
    > "$fixture/src/runtime/binary_object/bytecode_image/mod.rs"

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

echo "binary-object production boundary passed; all isolation canaries were rejected"
