#!/usr/bin/env bash
# Reproduce the complete classified outcome vector for RegExp lookbehind.

set -euo pipefail
export TZ=America/Los_Angeles

script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
root=$(CDPATH= cd -- "$script_dir/.." && pwd)
baseline=tests/test262-regexp-lookbehind-baseline.txt
manifest=tests/test262-regexp-lookbehind.txt
report=target/test262-regexp-lookbehind.tsv
json_report=target/test262-regexp-lookbehind.jsonl
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

expected_keys() {
    awk 'NF && $1 !~ /^#/ { print $0 "\tsloppy"; print $0 "\tstrict" }' "$manifest"
}

named_group_keys() {
    printf '%s\n' \
        'test/annexB/built-ins/RegExp/named-groups/non-unicode-malformed-lookbehind.js' \
        'test/built-ins/RegExp/named-groups/lookbehind.js' \
        | awk '{ print $0 "\tsloppy"; print $0 "\tstrict" }'
}

cd -- "$root"
if [[ ! -f "$baseline" ]]; then
    echo "error: lookbehind Test262 baseline is missing: $baseline" >&2
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
expected_r1n_profile=$(read_value r1n_oxide_profile_sha256)
expected_r1n_full_tsv=$(read_value r1n_full_tsv_sha256)
expected_r1n_keys=$(read_value r1n_keys_sha256)
expected_r1n_selected=$(read_value r1n_selected_sha256)
expected_r1n_variants=$(read_value r1n_variants)
expected_r1n_summary=$(read_value r1n_summary)
expected_tsv=$(read_value tsv_sha256)
expected_jsonl=$(read_value jsonl_sha256)
expected_summary=$(read_value summary)

if [[ "$expected_quickjs" != "2026-06-04" \
    || "$expected_test262" != "5c8206929d81b2d3d727ca6aac56c18358c8d790" \
    || "$expected_patch" != "f4b23b04641d438df0826fb17d7a5db276af2bdb085b42cc09aa8d50e0da9ba3" \
    || "$expected_config" != "79c64748ff1182baf5433d0a8378e3666738a785d02faf71f0d459ed42ae897b" \
    || "$expected_metadata" != "a37219960819e56a5c5c1723d31d6a33095c778bf5347385187fde96f927a06a" \
    || "$expected_profile" != "5c3c11f7c7c81fd54b706d6d50b5f28f6dddbd915c7b3543af9e5e6b5fb08aae" \
    || "$expected_schema" != "test262-canonical-classified-v2" \
    || "$expected_mode" != "both" \
    || "$timeout_ms" != "30000" \
    || "$expected_paths" != "27" \
    || "$expected_variants" != "54" \
    || "$expected_runnable" != "54" \
    || "$expected_passes" != "54" \
    || "$expected_manifest" != "6068e781c7b2185d2b3ccc402b59f6284e3429100dbf951adde680971d814fb7" \
    || "$expected_r1n_profile" != "6d5bb9a92d00babb6a4a0bcb19334fbcfcd532bb5382ce278ce85a960d40d781" \
    || "$expected_r1n_full_tsv" != "6035ae86888c4db9e99b73be65e706bf7b90ee83c108082a3e7931f2000edc61" \
    || "$expected_r1n_keys" != "6665dc1214d48e724a6c3ee3080b0b031c2470627f4a557e87dd1ce412e5c789" \
    || "$expected_r1n_selected" != "3e98efb5cb708fd84ffaada93ca444a35e4c79c82244eef11db293887c074e61" \
    || "$expected_r1n_variants" != "54" \
    || "$expected_r1n_summary" != "unsupported-feature=38 unsupported-negative-provenance=16" ]]; then
    echo "error: lookbehind R1n provenance metadata drifted" >&2
    exit 1
fi

actual_manifest_paths=$(awk 'NF && $1 !~ /^#/ { count++ } END { print count + 0 }' "$manifest")
unique_manifest_paths=$(awk 'NF && $1 !~ /^#/ { print }' "$manifest" | LC_ALL=C sort -u | wc -l | tr -d '[:space:]')
if [[ "$actual_manifest_paths" != "$expected_paths" \
    || "$unique_manifest_paths" != "$expected_paths" ]]; then
    echo "error: lookbehind manifest cardinality drifted" >&2
    echo "paths expected/actual/unique: $expected_paths / $actual_manifest_paths / $unique_manifest_paths" >&2
    exit 1
fi
if ! awk 'NF && $1 !~ /^#/ { print }' "$manifest" | LC_ALL=C sort -c; then
    echo "error: lookbehind manifest is not bytewise sorted" >&2
    exit 1
fi
actual_manifest=$(awk 'NF && $1 !~ /^#/ { print }' "$manifest" | sha256_stream)
if [[ "$actual_manifest" != "$expected_manifest" ]]; then
    echo "error: lookbehind manifest content drifted" >&2
    echo "expected: $expected_manifest" >&2
    echo "actual:   $actual_manifest" >&2
    exit 1
fi
actual_keys=$(expected_keys | LC_ALL=C sort | sha256_stream)
if [[ "$actual_keys" != "$expected_r1n_keys" ]]; then
    echo "error: lookbehind manifest variant keys drifted" >&2
    echo "expected: $expected_r1n_keys" >&2
    echo "actual:   $actual_keys" >&2
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
    echo "error: lookbehind Test262 baseline metadata drifted" >&2
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
    <(expected_keys | LC_ALL=C sort) \
    <(awk -F'\t' '!/^#/ && !($1 == "path" && $2 == "variant") { print $1 "\t" $2 }' "$report" | LC_ALL=C sort); then
    echo "error: lookbehind report keys drifted from the frozen manifest" >&2
    exit 1
fi

actual_summary=$(tail -n 1 "$report")
actual_passes=$(awk -F'\t' '!/^#/ && !($1 == "path" && $2 == "variant") && $7 == "pass" { count++ } END { print count + 0 }' "$report")
if [[ "$actual_summary" != "# summary $expected_summary" \
    || "$actual_passes" != "$expected_passes" ]]; then
    echo "error: lookbehind classified outcome counts drifted" >&2
    echo "summary expected/actual: # summary $expected_summary / $actual_summary" >&2
    echo "passes expected/actual:  $expected_passes / $actual_passes" >&2
    exit 1
fi

if ! diff -u \
    <(named_group_keys | LC_ALL=C sort) \
    <(awk -F'\t' '!/^#/ && !($1 == "path" && $2 == "variant") && $4 ~ /regexp-named-groups/ { print $1 "\t" $2 }' "$report" | LC_ALL=C sort); then
    echo "error: named-group lookbehind overlap drifted" >&2
    exit 1
fi
if ! awk -F'\t' '
    !/^#/ && !($1 == "path" && $2 == "variant") && $4 ~ /regexp-named-groups/ && $7 != "pass" { bad=1 }
    END { exit bad }
' "$report"; then
    echo "error: named-group lookbehind variants no longer pass" >&2
    exit 1
fi

actual_jsonl_lines=$(wc -l < "$json_report" | tr -d '[:space:]')
expected_jsonl_lines=$((expected_variants + 2))
actual_tsv=$(sha256_file "$report")
actual_jsonl=$(sha256_file "$json_report")
if [[ "$actual_jsonl_lines" != "$expected_jsonl_lines" \
    || "$actual_tsv" != "$expected_tsv" \
    || "$actual_jsonl" != "$expected_jsonl" ]]; then
    echo "error: lookbehind classified TSV/JSONL vector drifted" >&2
    echo "JSONL lines expected/actual: $expected_jsonl_lines / $actual_jsonl_lines" >&2
    echo "TSV expected:                $expected_tsv" >&2
    echo "TSV actual:                  $actual_tsv" >&2
    echo "JSONL expected:              $expected_jsonl" >&2
    echo "JSONL actual:                $actual_jsonl" >&2
    exit 1
fi

"$script_dir/check-rust-only.sh"
printf 'lookbehind Test262 vector matches: %s pass of %s variants across %s paths\n' \
    "$expected_passes" "$expected_variants" "$expected_paths"
