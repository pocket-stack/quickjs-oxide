#!/usr/bin/env bash
# Pin QuickJS 2026-06-04's static import-attributes loader2/checker contract.

set -euo pipefail
export LC_ALL=C

script_dir=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd)
root=$(CDPATH='' cd -- "$script_dir/.." && pwd)
source_file=$root/tests/fixtures/module_import_attributes_loader2.c
expected=$root/tests/fixtures/module_import_attributes_loader2.quickjs-2026-06-04.txt
source_sha256=d243a74b07d59d60f4d811c66447087547a43c5541baf60eb48b7d4c337e2863
expected_sha256=31121aad6e62055f7389392c83c09efaa225fa126b27bb9c08db6d8daca41e28
mode=${1:---check}

if [[ $# -gt 1 || "$mode" != --check && "$mode" != --validate ]]; then
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

verify_hash "$source_file" "$source_sha256"
verify_hash "$expected" "$expected_sha256"
if [[ "$mode" == --validate ]]; then
    echo 'module import-attributes oracle inputs are authenticated'
    exit 0
fi

oracle_dir=$("$script_dir/build-quickjs-oracle.sh" --test262-oracles)
[[ -f "$oracle_dir/quickjs.h" && -f "$oracle_dir/libquickjs.a" ]] \
    || die "pinned QuickJS headers/library are missing: $oracle_dir"
command -v "${CC:-cc}" >/dev/null 2>&1 || die "C compiler is missing: ${CC:-cc}"

tmp=$(mktemp -d "${TMPDIR:-/tmp}/quickjs-oxide-import-attrs-oracle.XXXXXX")
trap 'rm -rf -- "$tmp"' EXIT HUP INT TERM
link_flags=(-lm -lpthread)
if [[ "$(uname -s)" == Linux ]]; then
    link_flags+=(-ldl)
fi
"${CC:-cc}" -std=c11 -Wall -Wextra -Werror -Wno-unused-parameter \
    -I "$oracle_dir" "$source_file" "$oracle_dir/libquickjs.a" \
    "${link_flags[@]}" -o "$tmp/probe"
if ! "$tmp/probe" >"$tmp/actual" 2>"$tmp/stderr"; then
    sed -n '1,160p' "$tmp/stderr" >&2
    die 'pinned QuickJS import-attributes probe failed'
fi
[[ ! -s "$tmp/stderr" ]] || {
    sed -n '1,160p' "$tmp/stderr" >&2
    die 'pinned QuickJS import-attributes probe emitted stderr'
}
if ! cmp -s -- "$expected" "$tmp/actual"; then
    diff -u -- "$expected" "$tmp/actual" >&2 || true
    die 'pinned QuickJS import-attributes transcript drifted'
fi

echo 'module import-attributes oracle passed: QuickJS 2026-06-04 loader2 contract is stable'
