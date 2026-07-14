#!/usr/bin/env bash
# Prepare and verify the exact Test262 checkout shipped with the QuickJS oracle.

set -euo pipefail

script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
expected_commit=5c8206929d81b2d3d727ca6aac56c18358c8d790
expected_patch_sha256=f4b23b04641d438df0826fb17d7a5db276af2bdb085b42cc09aa8d50e0da9ba3
expected_config_sha256=79c64748ff1182baf5433d0a8378e3666738a785d02faf71f0d459ed42ae897b

oracle=$($script_dir/build-quickjs-oracle.sh)
if [[ "$oracle" != /* ]]; then
    oracle=$(CDPATH= cd -- "$(dirname -- "$oracle")" && pwd)/$(basename -- "$oracle")
fi
source_dir=$(dirname -- "$oracle")
suite=$source_dir/test262
patch=$source_dir/tests/test262.patch
config=$source_dir/test262.conf

if [[ ! -f "$patch" || ! -f "$config" ]]; then
    echo "error: pinned QuickJS Test262 patch or config is missing" >&2
    exit 1
fi

sha256_file() {
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum "$1" | awk '{print $1}'
    elif command -v shasum >/dev/null 2>&1; then
        shasum -a 256 "$1" | awk '{print $1}'
    else
        echo "error: sha256sum or shasum is required to verify Test262 inputs" >&2
        exit 2
    fi
}

actual_patch_sha256=$(sha256_file "$patch")
actual_config_sha256=$(sha256_file "$config")

if [[ "$actual_patch_sha256" != "$expected_patch_sha256" ]]; then
    echo "error: QuickJS Test262 patch checksum mismatch" >&2
    echo "expected: $expected_patch_sha256" >&2
    echo "actual:   $actual_patch_sha256" >&2
    exit 1
fi

if [[ "$actual_config_sha256" != "$expected_config_sha256" ]]; then
    echo "error: QuickJS Test262 config checksum mismatch" >&2
    echo "expected: $expected_config_sha256" >&2
    echo "actual:   $actual_config_sha256" >&2
    exit 1
fi

if [[ ! -e "$suite" ]]; then
    "${MAKE:-make}" -C "$source_dir" test2-bootstrap >&2
elif [[ ! -d "$suite/.git" ]]; then
    echo "error: Test262 path exists but is not the pinned git checkout: $suite" >&2
    exit 1
fi

if [[ ! -d "$suite/.git" ]]; then
    echo "error: QuickJS Test262 bootstrap did not create a git checkout: $suite" >&2
    exit 1
fi

actual_commit=$(git -C "$suite" rev-parse --verify 'HEAD^{commit}')
if [[ "$actual_commit" != "$expected_commit" ]]; then
    echo "error: Test262 checkout is not at the pinned QuickJS commit" >&2
    echo "expected: $expected_commit" >&2
    echo "actual:   $actual_commit" >&2
    exit 1
fi

expected_status=$' M harness/atomicsHelper.js\n M harness/regExpUtils.js'
actual_status=$(git -C "$suite" status --porcelain=v1 --untracked-files=all | LC_ALL=C sort)
if [[ "$actual_status" != "$expected_status" ]]; then
    echo "error: Test262 checkout contains changes other than the pinned QuickJS patch" >&2
    echo "expected status:" >&2
    printf '%s\n' "$expected_status" >&2
    echo "actual status:" >&2
    if [[ -n "$actual_status" ]]; then
        printf '%s\n' "$actual_status" >&2
    else
        echo "(clean; QuickJS patch is not applied)" >&2
    fi
    exit 1
fi

if ! git -C "$suite" apply --reverse --check "$patch"; then
    echo "error: pinned QuickJS Test262 patch cannot be reverse-applied" >&2
    exit 1
fi

if ! git -C "$suite" diff --no-ext-diff --no-color --no-renames \
    --abbrev=7 --src-prefix=a/ --dst-prefix=b/ -- \
    harness/atomicsHelper.js harness/regExpUtils.js | cmp -s - "$patch"; then
    echo "error: Test262 harness changes do not exactly match the pinned QuickJS patch" >&2
    exit 1
fi

CDPATH= cd -- "$suite"
pwd -P
