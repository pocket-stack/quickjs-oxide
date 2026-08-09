#!/usr/bin/env bash
# Freeze QuickJS 2026-06-04's JS_RunGC lifecycle while a native callback is
# re-entered from ordinary, generator, and pending-job JavaScript frames.

set -euo pipefail

script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
root=$(CDPATH= cd -- "$script_dir/.." && pwd)
fixture=$root/tests/fixtures/host_gc_reentrant.js
expected=$root/tests/fixtures/host_gc_reentrant.quickjs-2026-06-04.txt
fixture_sha256=0cb5dd063148070cd18507c51057a6664e3726705fca5af051b9f7486ee3d740
expected_sha256=4081eb2feb9f81b57c58beae27d863136ca69c94825687b09fe96edc5bbdf931
run_oxide=false

usage() {
    printf 'usage: %s [--check | --oxide]\n' "${0##*/}"
    printf '  --check  verify hashes and the pinned QuickJS transcript (default)\n'
    printf '  --oxide  also run the Rust integration differential\n'
}

case ${1-} in
    "" | --check) ;;
    --oxide) run_oxide=true ;;
    -h | --help)
        usage
        exit 0
        ;;
    *)
        usage >&2
        exit 2
        ;;
esac
if [[ $# -gt 1 ]]; then
    usage >&2
    exit 2
fi

sha256_file() {
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum "$1" | awk '{print $1}'
    elif command -v shasum >/dev/null 2>&1; then
        shasum -a 256 "$1" | awk '{print $1}'
    else
        echo 'error: sha256sum or shasum is required' >&2
        exit 2
    fi
}

verify_hash() {
    local path=$1
    local pinned=$2
    local actual
    if [[ ! -f "$path" ]]; then
        echo "error: host GC oracle input is missing: $path" >&2
        exit 1
    fi
    actual=$(sha256_file "$path")
    if [[ "$actual" != "$pinned" ]]; then
        echo "error: host GC oracle input hash drifted: $path" >&2
        echo "expected: $pinned" >&2
        echo "actual:   $actual" >&2
        exit 1
    fi
}

verify_hash "$fixture" "$fixture_sha256"
verify_hash "$expected" "$expected_sha256"

oracle_source=$("$script_dir/build-quickjs-oracle.sh" --test262-oracles)
oracle=$oracle_source/run-test262
if [[ ! -x "$oracle" ]]; then
    echo "error: pinned QuickJS run-test262 oracle is not executable: $oracle" >&2
    exit 1
fi

tmp_dir=$(mktemp -d "${TMPDIR:-/tmp}/quickjs-oxide-host-gc-oracle.XXXXXX")
trap 'rm -rf -- "$tmp_dir"' EXIT HUP INT TERM

if ! "$oracle" -N "$fixture" >"$tmp_dir/quickjs.out" 2>"$tmp_dir/quickjs.err"; then
    echo 'error: pinned QuickJS failed the host GC oracle fixture' >&2
    sed -n '1,200p' "$tmp_dir/quickjs.err" >&2
    exit 1
fi
if [[ -s "$tmp_dir/quickjs.err" ]]; then
    echo 'error: pinned QuickJS emitted unexpected host GC oracle stderr' >&2
    sed -n '1,200p' "$tmp_dir/quickjs.err" >&2
    exit 1
fi
if ! cmp -s -- "$expected" "$tmp_dir/quickjs.out"; then
    echo 'error: pinned QuickJS host GC transcript drifted' >&2
    diff -u -- "$expected" "$tmp_dir/quickjs.out" >&2 || true
    exit 1
fi

if [[ "$run_oxide" == true ]]; then
    cargo test --locked --manifest-path "$root/Cargo.toml" \
        --features test262-host \
        --test oracle_host_gc \
        test262_gc_reentry_matches_pinned_quickjs_lifecycle_transcript \
        -- --exact
    echo 'host GC differential passed: quickjs-oxide matches QuickJS 2026-06-04'
else
    echo 'host GC oracle passed: pinned QuickJS 2026-06-04 transcript is stable'
fi
