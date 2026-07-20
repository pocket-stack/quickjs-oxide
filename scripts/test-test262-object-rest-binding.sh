#!/usr/bin/env bash
# Reproduce the dependency-audited R2t object-rest binding Test262 gate.

set -euo pipefail
export TZ=America/Los_Angeles

script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
root=$(CDPATH= cd -- "$script_dir/.." && pwd)
baseline=tests/test262-object-rest-binding-baseline.txt
manifest=tests/test262-object-rest-binding.txt
profile=tests/test262-object-rest-binding.conf
report=target/test262-object-rest-binding.tsv
json_report=target/test262-object-rest-binding.jsonl
workers=${TEST262_WORKERS:-8}

read_value() {
    key=$1
    awk -F= -v key="$key" '
        $1 == key { sub(/^[^=]*=/, ""); print; found=1 }
        END { if (!found) exit 1 }
    ' "$baseline"
}

sha256_file() {
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum "$1" | awk '{print $1}'
    else
        shasum -a 256 "$1" | awk '{print $1}'
    fi
}

sha256_stream() {
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum | awk '{print $1}'
    else
        shasum -a 256 | awk '{print $1}'
    fi
}

read_header() {
    key=$1
    awk -F= -v key="$key" '
        $1 == "# " key { print $2; found=1 }
        END { if (!found) exit 1 }
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
expected_manifest=$(read_value manifest_sha256)
expected_manifest_file=$(read_value manifest_file_sha256)
expected_keys=$(read_value keys_sha256)
expected_nonpass=$(read_value nonpass_sha256)
expected_tsv=$(read_value tsv_sha256)
expected_jsonl=$(read_value jsonl_sha256)
expected_summary=$(read_value summary)

if [[ "$expected_quickjs" != "2026-06-04" \
    || "$expected_test262" != "5c8206929d81b2d3d727ca6aac56c18358c8d790" \
    || "$expected_patch" != "f4b23b04641d438df0826fb17d7a5db276af2bdb085b42cc09aa8d50e0da9ba3" \
    || "$expected_config" != "79c64748ff1182baf5433d0a8378e3666738a785d02faf71f0d459ed42ae897b" \
    || "$expected_metadata" != "a37219960819e56a5c5c1723d31d6a33095c778bf5347385187fde96f927a06a" \
    || "$expected_profile" != "122a2b055aaf40672a0540441861ecd1e6c09b65e88d45b947bc27a691afc45e" \
    || "$expected_schema" != "test262-canonical-classified-v2" \
    || "$expected_mode" != "both" \
    || "$expected_timeout_ms" != "30000" \
    || "$expected_paths" != "27" \
    || "$expected_variants" != "54" \
    || "$expected_runnable" != "54" \
    || "$expected_passes" != "54" \
    || "$expected_failures" != "0" \
    || "$expected_unsupported" != "0" \
    || "$expected_skipped" != "0" ]]; then
    echo "error: object-rest binding baseline metadata drifted" >&2
    exit 1
fi

actual_paths=$(awk 'NF && $1 !~ /^#/ { count++ } END { print count + 0 }' "$manifest")
unique_paths=$(awk 'NF && $1 !~ /^#/ { print }' "$manifest" | LC_ALL=C sort -u | wc -l | tr -d '[:space:]')
if [[ "$actual_paths" != "$expected_paths" || "$unique_paths" != "$expected_paths" ]]; then
    echo "error: object-rest binding manifest cardinality drifted" >&2
    exit 1
fi
awk 'NF && $1 !~ /^#/ { print }' "$manifest" | LC_ALL=C sort -c
actual_manifest=$(awk 'NF && $1 !~ /^#/ { print }' "$manifest" | sha256_stream)
if [[ "$actual_manifest" != "$expected_manifest" ]]; then
    echo "error: object-rest binding manifest content drifted" >&2
    exit 1
fi
if [[ "$(sha256_file "$manifest")" != "$expected_manifest_file" ]]; then
    echo "error: object-rest binding manifest file drifted" >&2
    exit 1
fi
if [[ "$(sha256_file "$profile")" != "$expected_profile" ]]; then
    echo "error: object-rest binding scoped capability profile drifted" >&2
    exit 1
fi
while IFS= read -r test_path; do
    if [[ ! -f "$suite/$test_path" ]]; then
        echo "error: pinned object-rest binding path is missing: $test_path" >&2
        exit 1
    fi
done < <(awk 'NF && $1 !~ /^#/ { print }' "$manifest")

rm -f -- "$report" "$json_report"
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

actual_variants=$(awk -F'\t' '!/^#/ && !($1 == "path" && $2 == "variant") { count++ } END { print count + 0 }' "$report")
execution_line=$(printf '%s\n' "$run_output" | awk '/^execution: runnable=/ { print; found=1 } END { if (!found) exit 1 }')
actual_runnable=${execution_line#*runnable=}
actual_runnable=${actual_runnable%% *}

if [[ "$(read_header quickjs)" != "$expected_quickjs" \
    || "$(read_header test262)" != "$expected_test262" \
    || "$(read_header test262_patch_sha256)" != "$expected_patch" \
    || "$(read_header test262_config_sha256)" != "$expected_config" \
    || "$(read_header test262_metadata_sha256)" != "$expected_metadata" \
    || "$(read_header oxide_profile_sha256)" != "$expected_profile" \
    || "$(read_header profile)" != "$expected_schema" \
    || "$(read_header mode)" != "$expected_mode" \
    || "$actual_variants" != "$expected_variants" \
    || "$actual_runnable" != "$expected_runnable" ]]; then
    echo "error: object-rest binding report metadata drifted" >&2
    exit 1
fi

# Every scoped feature must be exercised by at least one selected test, and
# every selected feature must be admitted. This prevents capability inflation.
diff -u \
    <(awk '
        /^\[features\]$/ { in_features=1; next }
        /^\[/ { in_features=0 }
        in_features && NF && $1 !~ /^#/ { print }
    ' "$profile" | LC_ALL=C sort) \
    <(awk -F'\t' '
        !/^#/ && !($1 == "path" && $2 == "variant") {
            count=split($4, features, ",")
            for (i=1; i <= count; i++) {
                if (features[i] != "") print features[i]
            }
        }
    ' "$report" | LC_ALL=C sort -u)

diff -u \
    <(awk 'NF && $1 !~ /^#/ { print }' "$manifest" | LC_ALL=C sort) \
    <(awk -F'\t' '!/^#/ && !($1 == "path" && $2 == "variant") { print $1 }' "$report" | LC_ALL=C sort -u)
actual_keys=$(awk -F'\t' '!/^#/ && !($1 == "path" && $2 == "variant") { print $1 "\t" $2 }' "$report" | LC_ALL=C sort | sha256_stream)
if [[ "$actual_keys" != "$expected_keys" ]]; then
    echo "error: object-rest binding path/variant key set drifted" >&2
    exit 1
fi

actual_passes=$(awk -F'\t' '!/^#/ && !($1 == "path" && $2 == "variant") && $7 == "pass" { count++ } END { print count + 0 }' "$report")
actual_unsupported=$(awk -F'\t' '!/^#/ && !($1 == "path" && $2 == "variant") && $7 ~ /^unsupported-/ { count++ } END { print count + 0 }' "$report")
actual_skipped=$(awk -F'\t' '!/^#/ && !($1 == "path" && $2 == "variant") && $7 ~ /^skipped-/ { count++ } END { print count + 0 }' "$report")
actual_failures=$((actual_variants - actual_passes - actual_unsupported - actual_skipped))
nonpass_count=$((actual_variants - actual_passes))
actual_nonpass=$(awk -F'\t' '!/^#/ && !($1 == "path" && $2 == "variant") && $7 != "pass" { print $1 "\t" $2 "\t" $7 "\t" $8 "\t" $9 "\t" $10 }' "$report" | sha256_stream)
if [[ "$actual_passes" != "$expected_passes" \
    || "$actual_failures" != "$expected_failures" \
    || "$actual_unsupported" != "$expected_unsupported" \
    || "$actual_skipped" != "$expected_skipped" \
    || "$actual_nonpass" != "$expected_nonpass" \
    || "$(tail -n 1 "$report")" != "# summary $expected_summary" \
    || "$(sha256_file "$report")" != "$expected_tsv" \
    || "$(sha256_file "$json_report")" != "$expected_jsonl" ]]; then
    echo "error: object-rest binding Test262 classified vector drifted" >&2
    if [[ "$nonpass_count" != 0 ]]; then
        printf 'path\tvariant\toutcome\tactual_phase\tactual_type\tdetail\n' >&2
        awk -F'\t' '
            !/^#/ && !($1 == "path" && $2 == "variant") && $7 != "pass" {
                print $1 "\t" $2 "\t" $7 "\t" $8 "\t" $9 "\t" $10
            }
        ' "$report" >&2
    fi
    exit 1
fi

printf 'object-rest binding Test262 gate passes: %s/%s variants across %s paths\n' \
    "$expected_passes" "$expected_variants" "$expected_paths"
