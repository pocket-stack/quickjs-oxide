#!/usr/bin/env bash
# Reproduce the complete classified outcome vector for the RegExp split slice.

set -euo pipefail
export TZ=America/Los_Angeles

script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
root=$(CDPATH= cd -- "$script_dir/.." && pwd)
suite=$("$script_dir/prepare-test262.sh")
source_dir=$(dirname -- "$suite")
baseline=tests/test262-regexp-split-baseline.txt
manifest=tests/test262-regexp-split.txt
report=target/test262-regexp-split.tsv
json_report=target/test262-regexp-split.jsonl
workers=${TEST262_WORKERS:-8}

read_value() {
    key=$1
    value=$(awk -F= -v key="$key" '$1 == key { sub(/^[^=]*=/, ""); print; found=1 } END { if (!found) exit 1 }' "$baseline")
    if [[ -z "$value" ]]; then
        echo "error: empty $key in $baseline" >&2
        exit 1
    fi
    printf '%s\n' "$value"
}

read_header() {
    key=$1
    awk -F= -v key="$key" '$1 == "# " key { print $2; found=1 } END { if (!found) exit 1 }' "$report"
}

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

sha256_stream() {
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum | awk '{print $1}'
    elif command -v shasum >/dev/null 2>&1; then
        shasum -a 256 | awk '{print $1}'
    else
        echo "error: sha256sum or shasum is required" >&2
        exit 2
    fi
}

cd -- "$root"
expected_quickjs=$(read_value quickjs)
expected_test262=$(read_value test262)
expected_patch=$(read_value test262_patch_sha256)
expected_config=$(read_value test262_config_sha256)
expected_metadata=$(read_value test262_metadata_sha256)
expected_profile=$(read_value oxide_profile_sha256)
timeout_ms=$(read_value timeout_ms)
expected_schema=$(read_value schema)
expected_mode=$(read_value mode)
expected_paths=$(read_value paths)
expected_variants=$(read_value variants)
expected_runnable=$(read_value runnable)
expected_passes=$(read_value passes)
expected_tsv=$(read_value tsv_sha256)
expected_jsonl=$(read_value jsonl_sha256)
expected_summary=$(read_value summary)
expected_manifest=$(read_value manifest_sha256)
expected_r1d_full_tsv=$(read_value r1d_full_tsv_sha256)
expected_r1d_selected=$(read_value r1d_selected_sha256)
expected_r1d_variants=$(read_value r1d_variants)
expected_r1d_summary=$(read_value r1d_summary)

if [[ "$expected_quickjs" != "2026-06-04" \
    || "$expected_test262" != "5c8206929d81b2d3d727ca6aac56c18358c8d790" \
    || "$expected_patch" != "f4b23b04641d438df0826fb17d7a5db276af2bdb085b42cc09aa8d50e0da9ba3" \
    || "$expected_config" != "79c64748ff1182baf5433d0a8378e3666738a785d02faf71f0d459ed42ae897b" \
    || "$expected_metadata" != "a37219960819e56a5c5c1723d31d6a33095c778bf5347385187fde96f927a06a" \
    || "$expected_profile" != "e2043efeaa2d8b4420d0c82550f7ba42d53588897ec14ac87f6f03c4358a8218" \
    || "$expected_schema" != "test262-canonical-classified-v2" \
    || "$expected_mode" != "both" \
    || "$timeout_ms" != "10000" \
    || "$expected_paths" != "46" \
    || "$expected_variants" != "92" \
    || "$expected_runnable" != "50" \
    || "$expected_passes" != "46" \
    || "$expected_manifest" != "fe5c9cc7b72022f45495237363505d534a2cc0e07a25c9c55dd9046f3f3ce9c6" \
    || "$expected_r1d_full_tsv" != "a695d6299b44e4298b553c28c12983b6b12fc9d8522f1216e18e16a6bad28012" \
    || "$expected_r1d_selected" != "d2bd57b0168a215151bedca53f2b852092c7f475545f45e46988bca1b34c231b" \
    || "$expected_r1d_variants" != "92" \
    || "$expected_r1d_summary" != "fail-runtime=42 pass=2 unsupported-feature=42 unsupported-host-create-realm=2 unsupported-parser=4" ]]; then
    echo "error: RegExp split R1d provenance metadata drifted" >&2
    exit 1
fi

actual_manifest_paths=$(awk 'NF && $1 !~ /^#/ { count++ } END { print count + 0 }' "$manifest")
unique_manifest_paths=$(awk 'NF && $1 !~ /^#/ { print }' "$manifest" | LC_ALL=C sort -u | wc -l | tr -d '[:space:]')
if [[ "$actual_manifest_paths" != "$expected_paths" \
    || "$unique_manifest_paths" != "$expected_paths" ]]; then
    echo "error: RegExp split manifest cardinality drifted" >&2
    echo "paths expected/actual/unique: $expected_paths / $actual_manifest_paths / $unique_manifest_paths" >&2
    exit 1
fi
if ! awk 'NF && $1 !~ /^#/ { print }' "$manifest" | LC_ALL=C sort -c; then
    echo "error: RegExp split manifest is not bytewise sorted" >&2
    exit 1
fi
actual_manifest=$(awk 'NF && $1 !~ /^#/ { print }' "$manifest" | sha256_stream)
if [[ "$actual_manifest" != "$expected_manifest" ]]; then
    echo "error: RegExp split manifest content drifted" >&2
    echo "expected: $expected_manifest" >&2
    echo "actual:   $actual_manifest" >&2
    exit 1
fi

rm -f -- "$report" "$json_report"
run_output=$(cargo run --locked --release --quiet --bin run-test262 -- \
    --suite "$suite" \
    --config "$source_dir/test262.conf" \
    --oxide-profile compat/test262-oxide.conf \
    --manifest "$manifest" \
    --report "$report" \
    --mode "$expected_mode" \
    --workers "$workers" \
    --timeout-ms "$timeout_ms" \
    --allow-failures)
printf '%s\n' "$run_output"

actual_quickjs=$(read_header quickjs)
actual_test262=$(read_header test262)
actual_patch=$(read_header test262_patch_sha256)
actual_config=$(read_header test262_config_sha256)
actual_metadata=$(read_header test262_metadata_sha256)
actual_profile=$(read_header oxide_profile_sha256)
actual_schema=$(read_header profile)
actual_mode=$(read_header mode)
actual_variants=$(awk -F'\t' '!/^#/ && !($1 == "path" && $2 == "variant") { count++ } END { print count + 0 }' "$report")
execution_line=$(printf '%s\n' "$run_output" | awk '/^execution: runnable=/ { print; found=1 } END { if (!found) exit 1 }')
actual_runnable=${execution_line#*runnable=}
actual_runnable=${actual_runnable%% *}

if [[ "$actual_quickjs" != "$expected_quickjs" \
    || "$actual_test262" != "$expected_test262" \
    || "$actual_patch" != "$expected_patch" \
    || "$actual_config" != "$expected_config" \
    || "$actual_metadata" != "$expected_metadata" \
    || "$actual_profile" != "$expected_profile" \
    || "$actual_schema" != "$expected_schema" \
    || "$actual_mode" != "$expected_mode" \
    || "$actual_variants" != "$expected_variants" \
    || "$actual_runnable" != "$expected_runnable" ]]; then
    echo "error: RegExp split Test262 baseline metadata drifted" >&2
    echo "quickjs expected/actual:  $expected_quickjs / $actual_quickjs" >&2
    echo "test262 expected/actual:  $expected_test262 / $actual_test262" >&2
    echo "patch expected/actual:    $expected_patch / $actual_patch" >&2
    echo "config expected/actual:   $expected_config / $actual_config" >&2
    echo "metadata expected/actual: $expected_metadata / $actual_metadata" >&2
    echo "profile expected/actual:  $expected_profile / $actual_profile" >&2
    echo "schema expected/actual:   $expected_schema / $actual_schema" >&2
    echo "mode expected/actual:     $expected_mode / $actual_mode" >&2
    echo "variants expected/actual: $expected_variants / $actual_variants" >&2
    echo "runnable expected/actual: $expected_runnable / $actual_runnable" >&2
    exit 1
fi

if ! diff -u \
    <(awk 'NF && $1 !~ /^#/ { print $0 "\tsloppy"; print $0 "\tstrict" }' "$manifest" | LC_ALL=C sort) \
    <(awk -F'\t' '!/^#/ && !($1 == "path" && $2 == "variant") { print $1 "\t" $2 }' "$report" | LC_ALL=C sort); then
    echo "error: RegExp split Test262 report keys drifted from the frozen manifest" >&2
    exit 1
fi

actual_summary=$(tail -n 1 "$report")
if [[ "$actual_summary" != "# summary $expected_summary" ]]; then
    echo "error: RegExp split Test262 classified summary drifted" >&2
    echo "expected: # summary $expected_summary" >&2
    echo "actual:   $actual_summary" >&2
    exit 1
fi
if [[ "$expected_passes" == 0 ]]; then
    if [[ " $expected_summary " == *" pass="* ]]; then
        echo "error: zero-pass RegExp split baseline unexpectedly records a pass outcome" >&2
        exit 1
    fi
elif [[ " $expected_summary " != *" pass=$expected_passes "* ]]; then
    echo "error: RegExp split pass count is inconsistent with the pinned summary" >&2
    exit 1
fi

actual_tsv=$(sha256_file "$report")
actual_jsonl=$(sha256_file "$json_report")
if [[ "$actual_tsv" != "$expected_tsv" || "$actual_jsonl" != "$expected_jsonl" ]]; then
    echo "error: RegExp split Test262 classified vector drifted" >&2
    echo "TSV expected:   $expected_tsv" >&2
    echo "TSV actual:     $actual_tsv" >&2
    echo "JSONL expected: $expected_jsonl" >&2
    echo "JSONL actual:   $actual_jsonl" >&2
    exit 1
fi

printf 'RegExp split Test262 vector matches: %s pass of %s variants across %s paths\n' \
    "$expected_passes" "$expected_variants" "$expected_paths"
