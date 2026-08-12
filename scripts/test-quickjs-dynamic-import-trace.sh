#!/usr/bin/env bash
# Verify the pinned QuickJS trace patch, protocol, and behavioral transparency.

set -euo pipefail
export LC_ALL=C

script_dir=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd)
root=$(CDPATH='' cd -- "$script_dir/.." && pwd)
builder=$script_dir/build-quickjs-dynamic-import-trace.sh
parser_test=$script_dir/test-parse-quickjs-dynamic-import-trace.mjs
fixtures=$root/tests/fixtures/dynamic-import-trace
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

verify_fixture() {
    local filename=$1
    local expected=$2
    local actual

    actual=$(sha256_file "$fixtures/$filename")
    [[ "$actual" == "$expected" ]] || die "fixture hash drifted: $filename"
}

verify_fixture agent.js 5b2c902fd0d24d3cb5a0f983998c8e06814eb77d03c20e6dfefda38603523da8
verify_fixture bare.js a2098bd92b10bf8b816d24b7556b1ce8c49a879d130489065ef1051c17e042f6
verify_fixture computed-block.js 79dac957fcab8f3f4389cd26ae217ec1bc860113305639b50bfa8cdc4f56402d
verify_fixture computed-invalid.js 1863811115d972ef74151c6ff2368fa16ee78a50a677fac3455b50adabac3752
verify_fixture computed-nested.js 839381da0f6489e2521507b329bd70805a74a8c3d8f84db794613a5fc3919a78
verify_fixture computed-template.js d0daf73ac3ded979d2d4ce35c8c06da7b30e5fe47750437ed838cb5a487aa052
verify_fixture root.js 6e4c0b8f0fbac19b8ea27677b1e69125fe280c790355b7dfeddcab046c780fe4

node "$parser_test"
node --check "$script_dir/parse-quickjs-dynamic-import-trace.mjs"
node --check "$parser_test"
bash -n "$builder"
bash -n "$0"

# The builder owns the authenticated archive/source/patch hashes. In validate
# mode, exercise its static declarations without requiring the archive itself.
patch_hash=$(sha256_file "$root/dev-support/quickjs/dynamic-import-trace-2026-06-04.patch")
grep -F "expected_patch_sha256=$patch_hash" "$builder" >/dev/null \
    || die 'builder trace patch hash is stale'
if [[ "$mode" == --validate ]]; then
    echo 'QuickJS dynamic-import trace oracle inputs are authenticated'
    exit 0
fi

tmp=$(mktemp -d "${TMPDIR:-/tmp}/quickjs-oxide-dynamic-import-trace-test.XXXXXX")
trap 'rm -rf -- "$tmp"' EXIT HUP INT TERM
build_root=$tmp/build
oracle_archive=${QJS_ORACLE_ARCHIVE:-"${QJS_ORACLE_CACHE:-$root/target/oracle}/quickjs-2026-06-04.tar.xz"}
trace_source=$(QJS_ORACLE_ARCHIVE="$oracle_archive" \
    QJS_DYNAMIC_IMPORT_TRACE_BUILD_DIR="$build_root" "$builder")
trace_runner=$trace_source/run-test262
stock_runner=$trace_source/run-test262.stock
root_fixture=$fixtures/root.js

run_capture() {
    local runner=$1
    local prefix=$2
    shift 2

    set +e
    "$@" "$runner" -T 1 -N --module "$root_fixture" \
        >"$tmp/$prefix.stdout" 2>"$tmp/$prefix.stderr"
    local rc=$?
    set -e
    printf '%s\n' "$rc" >"$tmp/$prefix.status"
}

run_capture "$stock_runner" stock env
run_capture "$trace_runner" traced env QJS_OXIDE_DYNAMIC_IMPORT_TRACE=1 \
    sh -c 'exec 3>"$1"; shift; exec "$@"' sh "$tmp/trace.tsv"
run_capture "$trace_runner" repeated env QJS_OXIDE_DYNAMIC_IMPORT_TRACE=1 \
    sh -c 'exec 3>"$1"; shift; exec "$@"' sh "$tmp/trace-repeated.tsv"

cmp -s "$tmp/stock.status" "$tmp/traced.status" \
    || die 'stock and trace exit statuses differ'
cmp -s "$tmp/stock.stdout" "$tmp/traced.stdout" \
    || die 'stock and trace stdout differ'
cmp -s "$tmp/stock.stderr" "$tmp/traced.stderr" \
    || die 'stock and trace stderr differ'
cmp -s "$tmp/traced.status" "$tmp/repeated.status" \
    || die 'repeated trace exit statuses differ'
cmp -s "$tmp/traced.stdout" "$tmp/repeated.stdout" \
    || die 'repeated trace stdout differs'
cmp -s "$tmp/traced.stderr" "$tmp/repeated.stderr" \
    || die 'repeated trace stderr differs'
cmp -s "$tmp/trace.tsv" "$tmp/trace-repeated.tsv" \
    || die 'repeated QJODI1 trace differs byte-for-byte'
[[ $(sed -n '1p' "$tmp/traced.status") == 0 ]] \
    || die 'synthetic dynamic-import fixture failed'

node "$parser_test" "$tmp/trace.tsv" "$root_fixture"

# A regular fd 3 is insufficient by itself: the dedicated environment opt-in
# is mandatory, and an unused descriptor must remain untouched.
set +e
sh -c 'exec 3>"$1"; shift; exec "$@"' sh "$tmp/no-opt-in.tsv" \
    "$trace_runner" -T 1 -N --module "$root_fixture" \
    >"$tmp/no-opt-in.stdout" 2>"$tmp/no-opt-in.stderr"
no_opt_in_status=$?
set -e
[[ $no_opt_in_status == 0 ]] || die 'non-opt-in run failed'
[[ ! -s "$tmp/no-opt-in.tsv" ]] || die 'fd 3 received trace data without opt-in'
cmp -s "$tmp/stock.stdout" "$tmp/no-opt-in.stdout" \
    || die 'non-opt-in stdout differs from stock'
cmp -s "$tmp/stock.stderr" "$tmp/no-opt-in.stderr" \
    || die 'non-opt-in stderr differs from stock'

set +e
QJS_OXIDE_DYNAMIC_IMPORT_TRACE=1 "$trace_runner" -T 1 -N --module \
    "$root_fixture" >"$tmp/no-fd.stdout" 2>"$tmp/no-fd.stderr"
no_fd_status=$?
set -e
[[ $no_fd_status != 0 ]] || die 'opt-in without fd 3 unexpectedly succeeded'
grep -F 'requires fd 3 to be a regular file' "$tmp/no-fd.stderr" >/dev/null \
    || die 'opt-in without fd 3 did not fail closed'

set +e
QJS_OXIDE_DYNAMIC_IMPORT_TRACE=1 \
    sh -c 'exec 3>"$1"; shift; exec "$@"' sh "$tmp/no-t1.tsv" \
    "$trace_runner" -N --module "$root_fixture" \
    >"$tmp/no-t1.stdout" 2>"$tmp/no-t1.stderr"
no_t1_status=$?
set -e
[[ $no_t1_status != 0 ]] || die 'trace without explicit -T 1 unexpectedly succeeded'
grep -F 'requires explicit -T 1' "$tmp/no-t1.stderr" >/dev/null \
    || die 'trace without explicit -T 1 did not fail closed'

set +e
QJS_OXIDE_DYNAMIC_IMPORT_TRACE=1 \
    sh -c 'exec 3>"$1"; shift; exec "$@"' sh "$tmp/agent.tsv" \
    "$trace_runner" -T 1 -N "$fixtures/agent.js" \
    >"$tmp/agent.stdout" 2>"$tmp/agent.stderr"
agent_status=$?
set -e
[[ $agent_status != 0 ]] || die 'trace with $262.agent unexpectedly succeeded'
grep -F 'does not support $262.agent' "$tmp/agent.stderr" >/dev/null \
    || die 'trace with $262.agent did not fail closed'

echo 'QuickJS dynamic-import trace oracle passed: authenticated A/B behavior and attributable QJODI1 events'
