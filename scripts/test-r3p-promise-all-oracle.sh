#!/usr/bin/env bash
# Freeze pinned QuickJS 2026-06-04 Promise.all behavior and optionally compare
# a quickjs-oxide qjs binary.

set -euo pipefail

script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
root=$(CDPATH= cd -- "$script_dir/.." && pwd)
fixture=$root/tests/fixtures/r3p_promise_all.js
expected=$root/tests/fixtures/r3p_promise_all.quickjs-2026-06-04.txt
fixture_sha256=e43406b9de7de5a88034ec5321486d7b352f2c6f43986fddba1b36fe79074835
expected_sha256=efb2fd9cfdd1db42291295e0b313dbf271b0007d30f3823e0377cb7196ab6b54
oxide=${OXIDE_QJS:-}

usage() {
    printf 'usage: %s [--check] [--oxide PATH]\n' "${0##*/}"
    printf '  --check       verify hashes and pinned QuickJS transcript (default)\n'
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
        echo "error: R3p oracle input is missing: $path" >&2
        exit 1
    fi
    actual=$(sha256_file "$path")
    if [[ "$actual" != "$pinned" ]]; then
        echo "error: R3p oracle input hash drifted: $path" >&2
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
        echo "error: $label failed to execute the R3p oracle fixture" >&2
        sed -n '1,200p' "$errors" >&2
        exit 1
    fi
    if [[ -s "$errors" ]]; then
        echo "error: $label emitted unexpected stderr" >&2
        sed -n '1,200p' "$errors" >&2
        exit 1
    fi
}

compare_transcript() {
    local label=$1
    local actual=$2
    if ! cmp -s -- "$expected" "$actual"; then
        echo "error: $label R3p transcript drifted" >&2
        diff -u -- "$expected" "$actual" >&2 || true
        exit 1
    fi
}

verify_hash "$fixture" "$fixture_sha256"
verify_hash "$expected" "$expected_sha256"

oracle=$("$script_dir/build-quickjs-oracle.sh")
if [[ ! -x "$oracle" ]]; then
    echo "error: pinned QuickJS oracle is not executable: $oracle" >&2
    exit 1
fi

tmp_dir=$(mktemp -d "${TMPDIR:-/tmp}/quickjs-oxide-r3p-oracle.XXXXXX")
trap 'rm -rf -- "$tmp_dir"' EXIT HUP INT TERM

{
    sed '$d' "$fixture"
    printf '\nr3pDone.then(\n'
    printf '    function () { print(r3pTranscript.join("\\n")); },\n'
    printf '    function (error) { throw error; }\n'
    printf ');\n'
} >"$tmp_dir/fixture.js"

run_engine "pinned QuickJS 2026-06-04" "$oracle" \
    "$tmp_dir/quickjs.out" "$tmp_dir/quickjs.err" --std --script "$tmp_dir/fixture.js"
compare_transcript "pinned QuickJS 2026-06-04" "$tmp_dir/quickjs.out"

if [[ -n "$oxide" ]]; then
    if [[ ! -x "$oxide" ]]; then
        echo "error: quickjs-oxide qjs is not executable: $oxide" >&2
        exit 2
    fi
    run_engine "quickjs-oxide" "$oxide" \
        "$tmp_dir/oxide.out" "$tmp_dir/oxide.err" "$tmp_dir/fixture.js"
    compare_transcript "quickjs-oxide" "$tmp_dir/oxide.out"
    if ! cmp -s -- "$tmp_dir/quickjs.out" "$tmp_dir/oxide.out"; then
        echo "error: quickjs-oxide differs from pinned QuickJS 2026-06-04" >&2
        diff -u -- "$tmp_dir/quickjs.out" "$tmp_dir/oxide.out" >&2 || true
        exit 1
    fi
    echo "R3p Promise.all differential passed: quickjs-oxide matches QuickJS 2026-06-04"
else
    echo "R3p Promise.all oracle passed: pinned QuickJS 2026-06-04 transcript is stable"
fi
