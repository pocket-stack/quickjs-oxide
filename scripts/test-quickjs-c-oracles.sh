#!/usr/bin/env bash
# Authenticate and run the pinned QuickJS C probes for public host/wire contracts.

set -euo pipefail
export LC_ALL=C

script_dir=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd)
root=$(CDPATH='' cd -- "$script_dir/.." && pwd)
manifest=$root/dev-support/quickjs-c-oracles.tsv
mode=${1:---check}

if [[ $# -gt 1 ]] || [[ "$mode" != --check && "$mode" != --validate ]]; then
    echo "usage: ${0##*/} [--check | --validate]" >&2
    exit 2
fi

die() {
    echo "error: $*" >&2
    exit 1
}

sha256_file() {
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum "$1" | awk '{print $1}'
    elif command -v shasum >/dev/null 2>&1; then
        shasum -a 256 "$1" | awk '{print $1}'
    else
        die 'sha256sum or shasum is required'
    fi
}

verify_hash() {
    local path=$1
    local expected_hash=$2
    local actual

    [[ -f "$path" && ! -L "$path" ]] || die "oracle input is missing: $path"
    actual=$(sha256_file "$path")
    if [[ "$actual" != "$expected_hash" ]]; then
        echo "error: oracle input hash drifted: $path" >&2
        echo "expected: $expected_hash" >&2
        echo "actual:   $actual" >&2
        exit 1
    fi
}

[[ -f "$manifest" && ! -L "$manifest" ]] || die "oracle manifest is missing: $manifest"
bash -n "$0"
expected_header=$'id\tfamily\tsource\tsource_sha256\texpected\texpected_sha256\tdescription'
header=$(sed -n '1p' "$manifest")
[[ "$header" == "$expected_header" ]] || die 'unsupported QuickJS C oracle manifest schema'
[[ "$(tail -c 1 "$manifest" | wc -l | tr -d '[:space:]')" == 1 ]] \
    || die 'QuickJS C oracle manifest must end with a newline'

ids=()
families=()
source_paths=()
sources=()
expected_paths=()
expected_files=()
descriptions=()
while IFS=$'\t' read -r id family source source_hash expected expected_hash description extra; do
    [[ -z "${extra:-}" ]] || die "manifest row has too many fields: $id"
    [[ -n "$id" && -n "$family" && -n "$source" && -n "$source_hash" \
        && -n "$expected" && -n "$expected_hash" && -n "$description" ]] \
        || die "manifest row is incomplete: ${id:-<empty>}"
    [[ "$id" =~ ^[a-z0-9-]+$ ]] || die "invalid oracle id: $id"
    [[ "$family" =~ ^[a-z0-9-]+$ ]] || die "invalid oracle family: $family"
    case $family in
        module)
            [[ "$source" =~ ^tests/fixtures/module_[a-z0-9_]+\.c$ ]] \
                || die "invalid module oracle source path: $source"
            ;;
        function-bytecode)
            [[ "$source" =~ ^tests/fixtures/function_bytecode_[a-z0-9_]+\.c$ ]] \
                || die "invalid function-bytecode oracle source path: $source"
            ;;
        *)
            die "unsupported oracle family: $family"
            ;;
    esac
    [[ "$expected" == "${source%.c}.quickjs-2026-06-04.txt" ]] \
        || die "oracle transcript does not match its source: $id"
    [[ "$source_hash" =~ ^[0-9a-f]{64}$ ]] || die "invalid source hash: $id"
    [[ "$expected_hash" =~ ^[0-9a-f]{64}$ ]] || die "invalid transcript hash: $id"
    for seen in "${ids[@]-}"; do
        [[ "$seen" != "$id" ]] || die "duplicate oracle id: $id"
    done
    for seen in "${source_paths[@]-}"; do
        [[ "$seen" != "$source" ]] || die "duplicate oracle source: $source"
    done
    for seen in "${expected_paths[@]-}"; do
        [[ "$seen" != "$expected" ]] || die "duplicate oracle transcript: $expected"
    done
    verify_hash "$root/$source" "$source_hash"
    verify_hash "$root/$expected" "$expected_hash"
    ids+=("$id")
    families+=("$family")
    source_paths+=("$source")
    sources+=("$root/$source")
    expected_paths+=("$expected")
    expected_files+=("$root/$expected")
    descriptions+=("$description")
done < <(tail -n +2 "$manifest")

for required_id in callback-contracts function-bytecode-ancestor-reference \
    function-bytecode-invalid-data-parent function-bytecode-nested-closure \
    function-bytecode-reference-boundary function-bytecode-wire \
    function-bytecode-writer-flags \
    import-attributes import-meta json; do
    found=false
    for id in "${ids[@]}"; do
        [[ "$id" != "$required_id" ]] || found=true
    done
    $found || die "QuickJS C oracle manifest lost baseline contract: $required_id"
done
[[ "$(printf '%s\n' "${ids[@]}" | LC_ALL=C sort)" == "$(printf '%s\n' "${ids[@]}")" ]] \
    || die 'QuickJS C oracle manifest must be sorted by id'

manifest_sources=$(printf '%s\n' "${source_paths[@]}" | LC_ALL=C sort)
fixture_sources=$(CDPATH='' cd -- "$root" && find tests/fixtures -maxdepth 1 \
    \( -type f -o -type l \) \
    \( -name 'module_*.c' -o -name 'function_bytecode_*.c' \) \
    -print | LC_ALL=C sort)
[[ "$manifest_sources" == "$fixture_sources" ]] \
    || die 'QuickJS C oracle manifest does not cover the complete source inventory'
manifest_transcripts=$(printf '%s\n' "${expected_paths[@]}" | LC_ALL=C sort)
fixture_transcripts=$(CDPATH='' cd -- "$root" && find tests/fixtures -maxdepth 1 \
    \( -type f -o -type l \) \
    \( -name 'module_*.quickjs-2026-06-04.txt' \
       -o -name 'function_bytecode_*.quickjs-2026-06-04.txt' \) \
    -print | LC_ALL=C sort)
[[ "$manifest_transcripts" == "$fixture_transcripts" ]] \
    || die 'QuickJS C oracle manifest does not cover the complete transcript inventory'
if [[ "$mode" == --validate ]]; then
    echo "QuickJS C oracle inputs are authenticated: ${#ids[@]} fixtures"
    exit 0
fi

oracle_dir=$("$script_dir/build-quickjs-oracle.sh" --test262-oracles)
[[ -f "$oracle_dir/quickjs.h" && -f "$oracle_dir/quickjs-libc.h" \
    && -f "$oracle_dir/libquickjs.a" ]] \
    || die "pinned QuickJS headers/library are missing: $oracle_dir"
command -v "${CC:-cc}" >/dev/null 2>&1 || die "C compiler is missing: ${CC:-cc}"

tmp=$(mktemp -d "${TMPDIR:-/tmp}/quickjs-oxide-c-oracles.XXXXXX")
trap 'rm -rf -- "$tmp"' EXIT HUP INT TERM
link_flags=(-lm -lpthread)
if [[ "$(uname -s)" == Linux ]]; then
    link_flags+=(-ldl)
fi

for index in "${!ids[@]}"; do
    id=${ids[$index]}
    probe=$tmp/$id
    actual=$tmp/$id.actual
    stderr=$tmp/$id.stderr
    "${CC:-cc}" -std=c11 -Wall -Wextra -Werror -Wno-unused-parameter \
        -I "$oracle_dir" "${sources[$index]}" "$oracle_dir/libquickjs.a" \
        "${link_flags[@]}" -o "$probe"
    if ! "$probe" >"$actual" 2>"$stderr"; then
        sed -n '1,200p' "$stderr" >&2
        die "pinned QuickJS C probe failed: $id"
    fi
    if [[ -s "$stderr" ]]; then
        sed -n '1,200p' "$stderr" >&2
        die "pinned QuickJS C probe emitted stderr: $id"
    fi
    if ! cmp -s -- "${expected_files[$index]}" "$actual"; then
        diff -u -- "${expected_files[$index]}" "$actual" >&2 || true
        die "pinned QuickJS C transcript drifted: $id"
    fi
    echo "QuickJS C oracle passed: $id [${families[$index]}] (${descriptions[$index]})"
done

echo "QuickJS C oracle suite passed: ${#ids[@]} QuickJS 2026-06-04 fixtures"
