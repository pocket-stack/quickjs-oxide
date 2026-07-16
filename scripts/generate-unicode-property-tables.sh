#!/usr/bin/env bash
# Regenerate exact Unicode property ranges by calling the checksum-pinned
# QuickJS 2026-06-04 libunicode implementation in a test-only generator.
# Product builds consume only the generated Rust arrays.

set -euo pipefail

if (( $# < 1 || $# > 2 )); then
    echo "usage: $0 /path/to/quickjs-2026-06-04 [output.rs]" >&2
    exit 2
fi

source_dir=$1
script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
root=$(CDPATH= cd -- "$script_dir/.." && pwd)
output_file=${2:-"$root/src/unicode_property_tables.rs"}
helper=$root/tests/fixtures/dump_unicode_properties.c

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
    echo "error: missing Unicode property generator helper: $helper" >&2
    exit 1
fi
if ! command -v cc >/dev/null 2>&1; then
    echo "error: cc is required for the test-only Unicode property generator" >&2
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

tmp_dir=$(mktemp -d "${TMPDIR:-/tmp}/quickjs-oxide-unicode-property.XXXXXX")
tmp_output=$(mktemp "${TMPDIR:-/tmp}/quickjs-oxide-unicode-property-output.XXXXXX")
trap 'rm -rf -- "$tmp_dir"; rm -f -- "$tmp_output"' EXIT HUP INT TERM

cc -std=c11 -O2 -I "$source_dir" \
    "$helper" "$source_dir/cutils.c" \
    -o "$tmp_dir/dump-unicode-properties"
"$tmp_dir/dump-unicode-properties" >"$tmp_output"

mkdir -p -- "$(dirname -- "$output_file")"
chmod 0644 "$tmp_output"
mv -- "$tmp_output" "$output_file"
trap 'rm -rf -- "$tmp_dir"' EXIT HUP INT TERM
