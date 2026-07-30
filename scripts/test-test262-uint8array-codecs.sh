#!/usr/bin/env bash
# Reproduce the pinned QuickJS differential and exact Uint8Array codec cohort.

set -euo pipefail
export TZ=America/Los_Angeles
export LC_ALL=C

script_dir=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd)
root=$(CDPATH='' cd -- "$script_dir/.." && pwd)
baseline=tests/test262-uint8array-codecs-baseline.txt
profile=tests/test262-uint8array-codecs.conf
manifest=tests/test262-uint8array-codecs.txt
report=target/test262-uint8array-codecs.tsv
json_report=target/test262-uint8array-codecs.jsonl
oracle_log=target/test262-uint8array-codecs-quickjs.log
workers=${TEST262_WORKERS:-8}

read_value() {
    local key=$1
    awk -F= -v key="$key" '
        $1 == key {
            if (found++) exit 2
            sub(/^[^=]*=/, "")
            print
        }
        END { if (found != 1) exit 1 }
    ' "$baseline"
}

sha256_file() {
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum "$1" | awk '{ print $1 }'
    else
        shasum -a 256 "$1" | awk '{ print $1 }'
    fi
}

sha256_stream() {
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum | awk '{ print $1 }'
    else
        shasum -a 256 | awk '{ print $1 }'
    fi
}

manifest_paths() {
    awk 'NF && $1 !~ /^#/ { print }' "$manifest"
}

profile_features() {
    awk '
        /^\[features\]$/ { inside=1; next }
        /^\[/ { inside=0 }
        inside && NF && $1 !~ /^#/ { print }
    ' "$profile"
}

report_rows() {
    awk -F'\t' '!/^#/ && !($1 == "path" && $2 == "variant") { print }' "$report"
}

read_header() {
    local key=$1
    awk -F= -v key="# $key" '
        $1 == key {
            if (found++) exit 2
            sub(/^[^=]*=/, "")
            print
        }
        END { if (found != 1) exit 1 }
    ' "$report"
}

cd -- "$root"
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
expected_timeout_ms=$(read_value timeout_ms)
expected_paths=$(read_value paths)
expected_variants=$(read_value variants)
expected_runnable=$(read_value runnable)
expected_passes=$(read_value passes)
expected_failures=$(read_value failures)
expected_unsupported=$(read_value unsupported)
expected_skipped=$(read_value skipped)
expected_features=$(read_value features)
expected_features_hash=$(read_value features_sha256)
expected_manifest=$(read_value manifest_sha256)
expected_manifest_file=$(read_value manifest_file_sha256)
expected_keys=$(read_value keys_sha256)
expected_nonpass=$(read_value nonpass_sha256)
expected_tsv=$(read_value tsv_sha256)
expected_jsonl=$(read_value jsonl_sha256)
expected_summary=$(read_value summary)
expected_oracle_variants=$(read_value quickjs_variants)

if [[ "$expected_quickjs" != "2026-06-04" \
    || "$expected_test262" != "5c8206929d81b2d3d727ca6aac56c18358c8d790" \
    || "$expected_patch" != "f4b23b04641d438df0826fb17d7a5db276af2bdb085b42cc09aa8d50e0da9ba3" \
    || "$expected_config" != "79c64748ff1182baf5433d0a8378e3666738a785d02faf71f0d459ed42ae897b" \
    || "$expected_metadata" != "a37219960819e56a5c5c1723d31d6a33095c778bf5347385187fde96f927a06a" \
    || "$expected_schema" != "test262-canonical-classified-v2" \
    || "$expected_mode" != "both" \
    || "$expected_timeout_ms" != "30000" \
    || "$expected_paths" != "69" \
    || "$expected_variants" != "138" \
    || "$expected_runnable" != "138" \
    || "$expected_passes" != "138" \
    || "$expected_failures" != "0" \
    || "$expected_unsupported" != "0" \
    || "$expected_skipped" != "0" \
    || "$expected_features" != "3" \
    || "$expected_oracle_variants" != "138" ]]; then
    echo "error: Uint8Array codec baseline metadata drifted" >&2
    exit 1
fi

actual_paths=$(manifest_paths)
if [[ "$(printf '%s\n' "$actual_paths" | wc -l | tr -d '[:space:]')" != "$expected_paths" \
    || "$(printf '%s\n' "$actual_paths" | LC_ALL=C sort -u | wc -l | tr -d '[:space:]')" != "$expected_paths" \
    || "$(printf '%s\n' "$actual_paths" | sha256_stream)" != "$expected_manifest" \
    || "$(sha256_file "$manifest")" != "$expected_manifest_file" ]]; then
    echo "error: Uint8Array codec manifest drifted" >&2
    exit 1
fi
printf '%s\n' "$actual_paths" | LC_ALL=C sort -c

actual_features=$(profile_features)
if [[ "$(printf '%s\n' "$actual_features" | wc -l | tr -d '[:space:]')" != "$expected_features" \
    || "$(printf '%s\n' "$actual_features" | sha256_stream)" != "$expected_features_hash" \
    || "$(sha256_file "$profile")" != "$expected_profile" ]]; then
    echo "error: Uint8Array codec capability profile drifted" >&2
    exit 1
fi

while IFS= read -r test_path; do
    [[ -f "$suite/$test_path" ]] || {
        echo "error: pinned Uint8Array codec path is missing: $test_path" >&2
        exit 1
    }
done <<<"$actual_paths"

QJS_ORACLE="$source_dir/qjs" cargo test --locked --test oracle_uint8array_codecs

run_output=$(cargo run --locked --release --quiet --bin run-test262 -- \
    --suite "$suite" \
    --config "$source_dir/test262.conf" \
    --oxide-profile "$profile" \
    --manifest "$manifest" \
    --report "$report" \
    --mode "$expected_mode" \
    --workers "$workers" \
    --timeout-ms "$expected_timeout_ms" \
    --allow-failures)
printf '%s\n' "$run_output"

actual_variants=$(report_rows | wc -l | tr -d '[:space:]')
actual_runnable=$(printf '%s\n' "$run_output" | awk '
    /^execution: runnable=/ {
        sub(/^execution: runnable=/, "")
        sub(/ .*/, "")
        print
        found=1
    }
    END { if (!found) exit 1 }
')
actual_passes=$(report_rows | awk -F'\t' '$7 == "pass" { count++ } END { print count + 0 }')
actual_failures=$(report_rows | awk -F'\t' '
    $7 != "pass" && $7 !~ /^unsupported-/ && $7 !~ /^skipped-/ { count++ }
    END { print count + 0 }
')
actual_unsupported=$(report_rows | awk -F'\t' '$7 ~ /^unsupported-/ { count++ } END { print count + 0 }')
actual_skipped=$(report_rows | awk -F'\t' '$7 ~ /^skipped-/ { count++ } END { print count + 0 }')
actual_keys=$(report_rows | awk -F'\t' '{ print $1 "\t" $2 }' | LC_ALL=C sort | sha256_stream)
actual_nonpass=$(report_rows | awk -F'\t' '
    $7 != "pass" { print $1 "\t" $2 "\t" $7 "\t" $8 "\t" $9 "\t" $10 }
' | sha256_stream)

if [[ "$(read_header quickjs)" != "$expected_quickjs" \
    || "$(read_header test262)" != "$expected_test262" \
    || "$(read_header test262_patch_sha256)" != "$expected_patch" \
    || "$(read_header test262_config_sha256)" != "$expected_config" \
    || "$(read_header test262_metadata_sha256)" != "$expected_metadata" \
    || "$(read_header oxide_profile_sha256)" != "$expected_profile" \
    || "$(read_header profile)" != "$expected_schema" \
    || "$(read_header mode)" != "$expected_mode" \
    || "$actual_variants" != "$expected_variants" \
    || "$actual_runnable" != "$expected_runnable" \
    || "$actual_passes" != "$expected_passes" \
    || "$actual_failures" != "$expected_failures" \
    || "$actual_unsupported" != "$expected_unsupported" \
    || "$actual_skipped" != "$expected_skipped" \
    || "$actual_keys" != "$expected_keys" \
    || "$actual_nonpass" != "$expected_nonpass" \
    || "$(tail -n 1 "$report")" != "# summary $expected_summary" \
    || "$(sha256_file "$report")" != "$expected_tsv" \
    || "$(sha256_file "$json_report")" != "$expected_jsonl" ]]; then
    echo "error: Uint8Array codec classified vector drifted" >&2
    report_rows | awk -F'\t' '$7 != "pass" { print }' | head -80 >&2
    exit 1
fi

oracle_files=()
while IFS= read -r test_path; do
    oracle_files+=("test262/$test_path")
done <<<"$actual_paths"
if ! (
    cd -- "$source_dir"
    ./run-test262 -m -c test262.conf -a -T "$workers" -f "${oracle_files[@]}"
) >"$root/$oracle_log" 2>&1; then
    echo "error: pinned QuickJS failed the Uint8Array codec manifest" >&2
    tail -80 "$oracle_log" >&2
    exit 1
fi
actual_oracle_variants=$(awk '
    /^Average memory statistics for [0-9]+ tests:$/ {
        print $5
        found=1
    }
    END { if (!found) exit 1 }
' "$oracle_log")
if [[ "$actual_oracle_variants" != "$expected_oracle_variants" ]]; then
    echo "error: pinned QuickJS Uint8Array codec variant count drifted" >&2
    exit 1
fi

printf 'Uint8Array codec gate passed: %s/%s variants in Oxide and pinned QuickJS\n' \
    "$actual_passes" "$actual_variants"
