#!/usr/bin/env bash
# Regenerate the Rust-only Unicode case-conversion tables from the
# checksum-pinned QuickJS 2026-06-04 oracle source. Product builds consume only
# the generated Rust arrays and never read, compile, or link the upstream C.

set -euo pipefail

if (( $# < 1 || $# > 2 )); then
    echo "usage: $0 /path/to/quickjs-2026-06-04/libunicode-table.h [output.rs]" >&2
    exit 2
fi

source_file=$1
tool_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
script_dir=$(CDPATH= cd -- "$tool_dir/.." && pwd)
output_file=${2:-"$script_dir/../src/source/unicode/generated/unicode/unicode_case_tables.rs"}
expected_sha256=cf782bc7a07549e976f606bd3cb8555858482b279574554dcb8d46412986006c
if command -v sha256sum >/dev/null 2>&1; then
    actual_sha256=$(sha256sum "$source_file" | awk '{ print $1 }')
elif command -v shasum >/dev/null 2>&1; then
    actual_sha256=$(shasum -a 256 "$source_file" | awk '{ print $1 }')
else
    echo "error: sha256sum or shasum is required to verify the Unicode source" >&2
    exit 2
fi
if [[ "$actual_sha256" != "$expected_sha256" ]]; then
    echo "error: unexpected libunicode-table.h checksum: $actual_sha256" >&2
    exit 1
fi

tmp_file=$(mktemp "${TMPDIR:-/tmp}/quickjs-oxide-unicode-case.XXXXXX")
trap 'rm -f -- "$tmp_file"' EXIT HUP INT TERM

awk -f "$tool_dir/case-tables.awk" "$source_file" >"$tmp_file"

mkdir -p -- "$(dirname -- "$output_file")"
chmod 0644 "$tmp_file"
mv -- "$tmp_file" "$output_file"
trap - EXIT HUP INT TERM
