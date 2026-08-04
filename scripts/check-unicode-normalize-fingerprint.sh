#!/usr/bin/env bash
# Recompute the Rust normalization tests' exhaustive fingerprints with the
# checksum-pinned QuickJS 2026-06-04 C implementation.

set -euo pipefail

if (( $# != 1 )); then
    echo "usage: $0 /path/to/quickjs-2026-06-04" >&2
    exit 2
fi

source_dir=$1
script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
root=$(CDPATH= cd -- "$script_dir/.." && pwd)
helper=$root/tests/fixtures/unicode_normalize_fingerprint.c

sha256_file() {
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum "$1" | awk '{ print $1 }'
    elif command -v shasum >/dev/null 2>&1; then
        shasum -a 256 "$1" | awk '{ print $1 }'
    else
        echo "error: sha256sum or shasum is required" >&2
        exit 2
    fi
}

verify_source() {
    file=$1
    expected=$2
    path=$source_dir/$file
    if [[ ! -f "$path" ]]; then
        echo "error: missing QuickJS Unicode source: $path" >&2
        exit 1
    fi
    actual=$(sha256_file "$path")
    if [[ "$actual" != "$expected" ]]; then
        echo "error: unexpected $file checksum: $actual" >&2
        exit 1
    fi
}

if [[ ! -f "$helper" ]]; then
    echo "error: missing normalization fingerprint helper: $helper" >&2
    exit 1
fi
if ! command -v cc >/dev/null 2>&1; then
    echo "error: cc is required for the normalization fingerprint oracle" >&2
    exit 2
fi

verify_source libunicode-table.h \
    cf782bc7a07549e976f606bd3cb8555858482b279574554dcb8d46412986006c
verify_source libunicode.c \
    26203ae888c0582e7d0e2113f13db0c9b39dc7b0b3836d68fa308c54f7a0898c
verify_source libunicode.h \
    ce310152bc80d7415dcb657e23abd9a40bf83e393c0d05d325dae384bb01d259
verify_source cutils.c \
    b73a403a59da30726257ddbdf5e399298941c1def997782ee0d4d33f796a80a2
verify_source cutils.h \
    d2da6d06a75b9e6c116c82b7a41df6bcc170c8b1779f374fa953ecf688eda647

tmp_dir=$(mktemp -d "${TMPDIR:-/tmp}/quickjs-oxide-unicode-normalize.XXXXXX")
trap 'rm -rf -- "$tmp_dir"' EXIT HUP INT TERM

cc -std=c11 -O2 -I "$source_dir" \
    "$helper" "$source_dir/cutils.c" \
    -o "$tmp_dir/unicode-normalize-fingerprint"

actual=$("$tmp_dir/unicode-normalize-fingerprint")
expected='canonical_count=2081
compatibility_count=5914
nonzero_cc_count=968
decomp_hash=6126396769325200388
cc_hash=2580281225329042492
compose_hash=17411631189690117515'

if [[ "$actual" != "$expected" ]]; then
    echo "error: pinned QuickJS normalization fingerprint changed" >&2
    diff -u <(printf '%s\n' "$expected") <(printf '%s\n' "$actual") >&2 || true
    exit 1
fi

printf '%s\n' "$actual"
