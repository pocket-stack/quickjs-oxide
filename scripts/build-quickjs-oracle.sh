#!/usr/bin/env bash
# Build the pinned upstream QuickJS release as a test-only differential oracle.

set -euo pipefail

script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
root=$(CDPATH= cd -- "$script_dir/.." && pwd)
version=2026-06-04
url=https://bellard.org/quickjs/quickjs-${version}.tar.xz
expected_sha256=b376e839b322978313d929fd20663b11ba58b75df5a46c126dd19ea2fa70ad2a
cache=${QJS_ORACLE_CACHE:-"$root/target/oracle"}
archive=$cache/quickjs-${version}.tar.xz
source_dir=$cache/quickjs-${version}
oracle=$source_dir/qjs

mkdir -p -- "$cache"

if [[ ! -f "$archive" ]]; then
    command -v curl >/dev/null 2>&1 || {
        echo "error: curl is required to download the QuickJS oracle" >&2
        exit 2
    }
    curl -fL "$url" -o "$archive"
fi

if command -v sha256sum >/dev/null 2>&1; then
    actual_sha256=$(sha256sum "$archive" | awk '{print $1}')
elif command -v shasum >/dev/null 2>&1; then
    actual_sha256=$(shasum -a 256 "$archive" | awk '{print $1}')
else
    echo "error: sha256sum or shasum is required to verify the oracle" >&2
    exit 2
fi

if [[ "$actual_sha256" != "$expected_sha256" ]]; then
    echo "error: QuickJS oracle archive checksum mismatch" >&2
    echo "expected: $expected_sha256" >&2
    echo "actual:   $actual_sha256" >&2
    exit 1
fi

if [[ ! -d "$source_dir" ]]; then
    tar -xJf "$archive" -C "$cache"
fi

if [[ ! -x "$oracle" ]]; then
    "${MAKE:-make}" -C "$source_dir" qjs >&2
fi

printf '%s\n' "$oracle"
