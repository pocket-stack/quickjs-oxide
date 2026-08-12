#!/usr/bin/env bash
# Build the pinned, test-only QuickJS dynamic-import trace runner in isolation.

set -euo pipefail
export LC_ALL=C

script_dir=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd)
root=$(CDPATH='' cd -- "$script_dir/.." && pwd)
version=2026-06-04
archive=${QJS_ORACLE_ARCHIVE:-"$root/target/oracle/quickjs-${version}.tar.xz"}
patch_file=$root/dev-support/quickjs/dynamic-import-trace-${version}.patch
expected_archive_sha256=b376e839b322978313d929fd20663b11ba58b75df5a46c126dd19ea2fa70ad2a
expected_patch_sha256=9a5cf244f8573c9c64773d22f0d4a88159f4b1e72db1737a2a885f4fd605b421
expected_quickjs_sha256=a00762d2eee42316cecbc9c15efc4549b715ec500461845cd91b2a6c38190d08
expected_runner_sha256=58f37e301f36b2b630a247d59105fc373953340c691667b28822cd703d8f203e
expected_makefile_sha256=9a3a4e4021203322a957b15aa181c951949a02a01e06dafbf2296cbefb06aa2b
expected_patched_quickjs_sha256=c9b45b64f4bbcbb58042c52e05f5f1efeb87184c7404d77ff4fc354ab65a073f
expected_patched_runner_sha256=2a6d4a1fb8d77d389f92a6dd62b3b96bc4a7e6a1a5312973542fa5d829eef912

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
    local expected=$2
    local label=$3
    local actual

    [[ -f "$path" && ! -L "$path" ]] || die "$label is not a regular file: $path"
    actual=$(sha256_file "$path")
    if [[ "$actual" != "$expected" ]]; then
        echo "error: $label checksum mismatch" >&2
        echo "expected: $expected" >&2
        echo "actual:   $actual" >&2
        exit 1
    fi
}

for command_name in tar patch "${MAKE:-make}"; do
    command -v "$command_name" >/dev/null 2>&1 || die "$command_name is required"
done

verify_hash "$archive" "$expected_archive_sha256" 'QuickJS archive'
verify_hash "$patch_file" "$expected_patch_sha256" 'trace patch'

umask 077
if [[ -n ${QJS_DYNAMIC_IMPORT_TRACE_BUILD_DIR:-} ]]; then
    work_dir=$QJS_DYNAMIC_IMPORT_TRACE_BUILD_DIR
    [[ ! -e "$work_dir" && ! -L "$work_dir" ]] \
        || die "trace build directory already exists: $work_dir"
    mkdir -p -- "$work_dir"
else
    work_dir=$(mktemp -d "${TMPDIR:-/tmp}/quickjs-oxide-dynamic-import-trace.XXXXXX")
fi

complete=0
cleanup() {
    if [[ $complete -eq 0 ]]; then
        rm -rf -- "$work_dir" 2>/dev/null || true
    fi
}
trap cleanup EXIT
trap 'exit 129' HUP
trap 'exit 130' INT
trap 'exit 143' TERM

mkdir -- "$work_dir/stock" "$work_dir/trace"
tar -xJf "$archive" -C "$work_dir/stock"
tar -xJf "$archive" -C "$work_dir/trace"
stock_source=$work_dir/stock/quickjs-${version}
source_dir=$work_dir/trace/quickjs-${version}
[[ -d "$source_dir" && ! -L "$source_dir" ]] \
    || die 'verified archive did not extract the expected source directory'
[[ -d "$stock_source" && ! -L "$stock_source" ]] \
    || die 'verified archive did not extract the stock source directory'

verify_hash "$source_dir/quickjs.c" "$expected_quickjs_sha256" 'QuickJS quickjs.c'
verify_hash "$source_dir/run-test262.c" "$expected_runner_sha256" 'QuickJS run-test262.c'
verify_hash "$source_dir/Makefile" "$expected_makefile_sha256" 'QuickJS Makefile'
verify_hash "$stock_source/quickjs.c" "$expected_quickjs_sha256" \
    'stock QuickJS quickjs.c'
verify_hash "$stock_source/run-test262.c" "$expected_runner_sha256" \
    'stock QuickJS run-test262.c'

"${MAKE:-make}" -C "$stock_source" -j "${QJS_DYNAMIC_IMPORT_TRACE_JOBS:-2}" \
    run-test262 >&2
[[ -f "$stock_source/run-test262" && -x "$stock_source/run-test262" ]] \
    || die 'stock run-test262 binary was not produced'

patch -d "$source_dir" -p1 -F 0 -f -i "$patch_file" >&2
verify_hash "$source_dir/quickjs.c" "$expected_patched_quickjs_sha256" \
    'patched QuickJS quickjs.c'
verify_hash "$source_dir/run-test262.c" "$expected_patched_runner_sha256" \
    'patched QuickJS run-test262.c'

"${MAKE:-make}" -C "$source_dir" -j "${QJS_DYNAMIC_IMPORT_TRACE_JOBS:-2}" \
    run-test262 >&2
[[ -f "$source_dir/run-test262" && -x "$source_dir/run-test262" ]] \
    || die 'trace run-test262 binary was not produced'
cp -- "$stock_source/run-test262" "$source_dir/run-test262.stock"

complete=1
printf '%s\n' "$source_dir"
