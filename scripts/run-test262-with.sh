#!/usr/bin/env bash
# Reproduce the focused classified `with` statement vector.

set -euo pipefail
export TZ=America/Los_Angeles

script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
root=$(CDPATH= cd -- "$script_dir/.." && pwd)
baseline=tests/test262-with-baseline.txt
manifest=tests/test262-with.txt
report=target/test262-with.tsv
json_report=target/test262-with.jsonl
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
if [[ ! -f "$baseline" || ! -f "$manifest" ]]; then
    echo "error: with-statement Test262 manifest or baseline is missing" >&2
    exit 1
fi
suite=$("$script_dir/prepare-test262.sh")
source_dir=$(dirname -- "$suite")

expected_quickjs=$(read_value quickjs)
expected_test262=$(read_value test262)
expected_patch=$(read_value test262_patch_sha256)
expected_config=$(read_value test262_config_sha256)
expected_metadata=$(read_value test262_metadata_sha256)
expected_profile=$(read_value oxide_profile_sha256)
expected_schema=$(read_value schema)
expected_mode=$(read_value mode)
timeout_ms=$(read_value timeout_ms)
expected_paths=$(read_value paths)
expected_variants=$(read_value variants)
expected_runnable=$(read_value runnable)
expected_passes=$(read_value passes)
expected_manifest=$(read_value manifest_sha256)
expected_keys_hash=$(read_value keys_sha256)
expected_nonpass=$(read_value nonpass_sha256)
expected_tsv=$(read_value tsv_sha256)
expected_jsonl=$(read_value jsonl_sha256)
expected_summary=$(read_value summary)

if [[ "$expected_quickjs" != "2026-06-04" \
    || "$expected_test262" != "5c8206929d81b2d3d727ca6aac56c18358c8d790" \
    || "$expected_patch" != "f4b23b04641d438df0826fb17d7a5db276af2bdb085b42cc09aa8d50e0da9ba3" \
    || "$expected_config" != "79c64748ff1182baf5433d0a8378e3666738a785d02faf71f0d459ed42ae897b" \
    || "$expected_metadata" != "a37219960819e56a5c5c1723d31d6a33095c778bf5347385187fde96f927a06a" \
    || "$expected_profile" != "6a4d3dc37da05f6e63d7b8564483159c383ed66c665a2b5530624e628f73b908" \
    || "$expected_schema" != "test262-canonical-classified-v2" \
    || "$expected_mode" != "both" \
    || "$timeout_ms" != "30000" \
    || "$expected_paths" != "203" \
    || "$expected_variants" != "205" \
    || "$expected_runnable" != "205" ]]; then
    echo "error: with-statement baseline metadata drifted" >&2
    exit 1
fi
if ! [[ "$expected_passes" =~ ^[0-9]+$ ]] \
    || (( expected_passes > expected_runnable )); then
    echo "error: with-statement pass baseline is invalid" >&2
    exit 1
fi

actual_manifest_paths=$(awk 'NF && $1 !~ /^#/ { count++ } END { print count + 0 }' "$manifest")
unique_manifest_paths=$(awk 'NF && $1 !~ /^#/ { print }' "$manifest" | LC_ALL=C sort -u | wc -l | tr -d '[:space:]')
if [[ "$actual_manifest_paths" != "$expected_paths" \
    || "$unique_manifest_paths" != "$expected_paths" ]]; then
    echo "error: with-statement manifest cardinality drifted" >&2
    exit 1
fi
if ! awk 'NF && $1 !~ /^#/ { print }' "$manifest" | LC_ALL=C sort -c; then
    echo "error: with-statement manifest is not bytewise sorted" >&2
    exit 1
fi
actual_manifest=$(awk 'NF && $1 !~ /^#/ { print }' "$manifest" | sha256_stream)
if [[ "$actual_manifest" != "$expected_manifest" ]]; then
    echo "error: with-statement manifest content drifted" >&2
    exit 1
fi
while IFS= read -r test_path; do
    if [[ ! -f "$suite/$test_path" ]]; then
        echo "error: pinned Test262 path is missing: $test_path" >&2
        exit 1
    fi
done < <(awk 'NF && $1 !~ /^#/ { print }' "$manifest")

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
    echo "error: with-statement Test262 report metadata drifted" >&2
    exit 1
fi

if ! diff -u \
    <(awk 'NF && $1 !~ /^#/ { print }' "$manifest" | LC_ALL=C sort) \
    <(awk -F'\t' '!/^#/ && !($1 == "path" && $2 == "variant") { print $1 }' "$report" | LC_ALL=C sort -u); then
    echo "error: with-statement report paths drifted from the frozen manifest" >&2
    exit 1
fi
actual_keys_hash=$(awk -F'\t' '!/^#/ && !($1 == "path" && $2 == "variant") { print $1 "\t" $2 }' "$report" | LC_ALL=C sort | sha256_stream)
actual_summary=$(tail -n 1 "$report")
actual_passes=$(awk -F'\t' '!/^#/ && !($1 == "path" && $2 == "variant") && $7 == "pass" { count++ } END { print count + 0 }' "$report")
actual_nonpass=$(awk -F'\t' '!/^#/ && !($1 == "path" && $2 == "variant") && $7 != "pass" { print $1 "\t" $2 "\t" $7 "\t" $8 "\t" $9 "\t" $10 }' "$report" | sha256_stream)
if [[ "$actual_keys_hash" != "$expected_keys_hash" \
    || "$actual_summary" != "# summary $expected_summary" \
    || "$actual_passes" != "$expected_passes" \
    || "$actual_nonpass" != "$expected_nonpass" ]]; then
    echo "error: with-statement classified outcomes drifted" >&2
    exit 1
fi
actual_jsonl_lines=$(wc -l < "$json_report" | tr -d '[:space:]')
expected_jsonl_lines=$((expected_variants + 2))
actual_tsv=$(sha256_file "$report")
actual_jsonl=$(sha256_file "$json_report")
if [[ "$actual_jsonl_lines" != "$expected_jsonl_lines" \
    || "$actual_tsv" != "$expected_tsv" \
    || "$actual_jsonl" != "$expected_jsonl" ]]; then
    echo "error: with-statement classified TSV/JSONL vector drifted" >&2
    exit 1
fi

"$script_dir/check-rust-only.sh"
printf 'with-statement Test262 vector matches: %s pass of %s variants across %s paths\n' \
    "$expected_passes" "$expected_variants" "$expected_paths"
