#!/usr/bin/env bash
# Freeze the QuickJS 2026-06-04 ordinary private-method and brand semantics
# and, optionally, compare a quickjs-oxide qjs binary against the transcript.

set -euo pipefail

script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
root=$(CDPATH= cd -- "$script_dir/.." && pwd)
fixture=$root/tests/fixtures/r3i_class_private_methods.js
expected=$root/tests/fixtures/r3i_class_private_methods.quickjs-2026-06-04.txt
fixture_sha256=23053aea3d41c9ee72a61007c713a17d7082dd418c9b06433a03800173b77567
expected_sha256=7e87481d5b8a4202554d7c50264bb8063547512468f8c2df22bf05d06965e452
oxide=${OXIDE_QJS:-}

usage() {
    printf 'usage: %s [--check] [--oxide PATH]\n' "${0##*/}"
    printf '  --check       verify fixture hashes and the pinned QuickJS transcript (default)\n'
    printf '  --oxide PATH  additionally require byte-for-byte quickjs-oxide parity\n'
}

case ${1-} in
    "" | --check) ;;
    --oxide)
        if [[ $# -ne 2 ]]; then
            usage >&2
            exit 2
        fi
        oxide=$2
        ;;
    -h | --help)
        usage
        exit 0
        ;;
    *)
        usage >&2
        exit 2
        ;;
esac
if [[ ${1-} != --oxide && $# -gt 1 ]]; then
    usage >&2
    exit 2
fi

sha256_file() {
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum "$1" | awk '{print $1}'
    elif command -v shasum >/dev/null 2>&1; then
        shasum -a 256 "$1" | awk '{print $1}'
    else
        echo "error: sha256sum or shasum is required" >&2
        exit 2
    fi
}

verify_hash() {
    local path=$1
    local pinned=$2
    local actual
    if [[ ! -f "$path" ]]; then
        echo "error: R3i oracle input is missing: $path" >&2
        exit 1
    fi
    actual=$(sha256_file "$path")
    if [[ "$actual" != "$pinned" ]]; then
        echo "error: R3i oracle input hash drifted: $path" >&2
        echo "expected: $pinned" >&2
        echo "actual:   $actual" >&2
        exit 1
    fi
}

run_engine() {
    local label=$1
    local engine=$2
    local output=$3
    local errors=$4
    shift 4
    if ! "$engine" "$@" >"$output" 2>"$errors"; then
        echo "error: $label failed to execute the R3i oracle fixture" >&2
        sed -n '1,160p' "$errors" >&2
        exit 1
    fi
    if [[ -s "$errors" ]]; then
        echo "error: $label emitted unexpected stderr" >&2
        sed -n '1,160p' "$errors" >&2
        exit 1
    fi
}

compare_transcript() {
    local label=$1
    local actual=$2
    if ! cmp -s -- "$expected" "$actual"; then
        echo "error: $label R3i transcript drifted" >&2
        diff -u -- "$expected" "$actual" >&2 || true
        exit 1
    fi
}

verify_hash "$fixture" "$fixture_sha256"
verify_hash "$expected" "$expected_sha256"

oracle=$($script_dir/build-quickjs-oracle.sh)
if [[ ! -x "$oracle" ]]; then
    echo "error: pinned QuickJS oracle is not executable: $oracle" >&2
    exit 1
fi

tmp_dir=$(mktemp -d "${TMPDIR:-/tmp}/quickjs-oxide-r3i-oracle.XXXXXX")
trap 'rm -rf -- "$tmp_dir"' EXIT HUP INT TERM

{
    cat "$fixture"
    printf '\nprint(r3iTranscript.join("\\n"));\n'
} >"$tmp_dir/quickjs-fixture.js"

run_engine "pinned QuickJS 2026-06-04" "$oracle" \
    "$tmp_dir/quickjs.out" "$tmp_dir/quickjs.err" --script "$tmp_dir/quickjs-fixture.js"
compare_transcript "pinned QuickJS 2026-06-04" "$tmp_dir/quickjs.out"

if [[ -n "$oxide" ]]; then
    if [[ ! -x "$oxide" ]]; then
        echo "error: quickjs-oxide qjs is not executable: $oxide" >&2
        exit 2
    fi
    run_engine "quickjs-oxide" "$oxide" \
        "$tmp_dir/oxide.out" "$tmp_dir/oxide.err" --print-result "$fixture"
    compare_transcript "quickjs-oxide" "$tmp_dir/oxide.out"
    if ! cmp -s -- "$tmp_dir/quickjs.out" "$tmp_dir/oxide.out"; then
        echo "error: quickjs-oxide differs from pinned QuickJS 2026-06-04" >&2
        diff -u -- "$tmp_dir/quickjs.out" "$tmp_dir/oxide.out" >&2 || true
        exit 1
    fi
    echo "R3i class private-method differential passed: quickjs-oxide matches QuickJS 2026-06-04"
else
    echo "R3i class private-method oracle passed: pinned QuickJS 2026-06-04 transcript is stable"
fi
