#!/usr/bin/env bash
# Shared reproducibility gate for the dependency-audited R2v object assignment cohorts.

set -euo pipefail
export TZ=America/Los_Angeles

if [[ $# -ne 1 ]]; then
    echo "usage: $0 flat|nested|rest" >&2
    exit 2
fi

cohort=$1
case "$cohort" in
    flat|nested|rest) ;;
    *)
        echo "error: unknown object assignment cohort: $cohort" >&2
        exit 2
        ;;
esac

script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
root=$(CDPATH= cd -- "$script_dir/.." && pwd)
stem="test262-object-assignment-$cohort"
baseline="tests/$stem-baseline.txt"
manifest="tests/$stem.txt"
profile="tests/$stem.conf"
report="target/$stem.tsv"
json_report="target/$stem.jsonl"
workers=${TEST262_WORKERS:-8}

read_value() {
    local key=$1
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
    local key=$1
    awk -F= -v key="$key" '
        $1 == "# " key { print $2; found=1 }
        END { if (!found) exit 1 }
    ' "$report"
}

profile_section() {
    local section=$1
    awk -v section="[$section]" '
        $0 == section { inside=1; next }
        /^\[/ { inside=0 }
        inside && NF && $1 !~ /^#/ { print }
    ' "$profile"
}

features_admitted() {
    local features=$1 feature
    IFS=',' read -r -a feature_list <<< "$features"
    for feature in "${feature_list[@]}"; do
        feature=${feature//[[:space:]]/}
        case "$cohort:$feature" in
            flat:Symbol|flat:const|flat:destructuring-binding|flat:let) ;;
            nested:destructuring-binding) ;;
            rest:Symbol|rest:destructuring-binding|rest:object-rest) ;;
            *) return 1 ;;
        esac
    done
}

derive_manifest() {
    local test_file basename features
    case "$cohort" in
        flat)
            for test_file in "$suite"/test/language/expressions/assignment/dstr/obj-*.js; do
                basename=${test_file##*/}
                if [[ "$basename" =~ nested-(array|obj) || "$basename" == *rest* ]]; then
                    continue
                fi
                features=$(sed -n 's/^features: \[\(.*\)\]$/\1/p' "$test_file")
                if features_admitted "$features"; then
                    printf 'test/language/expressions/assignment/dstr/%s\n' "$basename"
                fi
            done
            ;;
        nested)
            for test_file in "$suite"/test/language/expressions/assignment/dstr/obj-prop-nested-*.js; do
                basename=${test_file##*/}
                features=$(sed -n 's/^features: \[\(.*\)\]$/\1/p' "$test_file")
                if features_admitted "$features"; then
                    printf 'test/language/expressions/assignment/dstr/%s\n' "$basename"
                fi
            done
            ;;
        rest)
            for test_file in "$suite"/test/language/expressions/assignment/dstr/obj-rest-*.js; do
                basename=${test_file##*/}
                features=$(sed -n 's/^features: \[\(.*\)\]$/\1/p' "$test_file")
                if features_admitted "$features"; then
                    printf 'test/language/expressions/assignment/dstr/%s\n' "$basename"
                fi
            done
            ;;
    esac | LC_ALL=C sort
}

derive_negative_tests() {
    local test_path
    while IFS= read -r test_path; do
        if sed -n '/^\/\*---$/,/^---\*\/$/p' "$suite/$test_path" | grep -q '^negative:'; then
            printf '%s\n' "$test_path"
        fi
    done < <(awk 'NF && $1 !~ /^#/ { print }' "$manifest")
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

case "$cohort" in
    flat) required_paths=67 ;;
    nested) required_paths=14 ;;
    rest) required_paths=26 ;;
esac

if [[ "$expected_quickjs" != "2026-06-04" \
    || "$expected_test262" != "5c8206929d81b2d3d727ca6aac56c18358c8d790" \
    || "$expected_patch" != "f4b23b04641d438df0826fb17d7a5db276af2bdb085b42cc09aa8d50e0da9ba3" \
    || "$expected_config" != "79c64748ff1182baf5433d0a8378e3666738a785d02faf71f0d459ed42ae897b" \
    || "$expected_metadata" != "a37219960819e56a5c5c1723d31d6a33095c778bf5347385187fde96f927a06a" \
    || "$expected_schema" != "test262-canonical-classified-v2" \
    || "$expected_mode" != "both" \
    || "$expected_timeout_ms" != "30000" \
    || "$expected_paths" != "$required_paths" \
    || "$expected_variants" != "$expected_runnable" \
    || "$expected_passes" != "$expected_runnable" \
    || "$expected_failures" != "0" \
    || "$expected_unsupported" != "0" \
    || "$expected_skipped" != "0" ]]; then
    echo "error: $cohort object assignment baseline metadata drifted" >&2
    exit 1
fi

actual_paths=$(awk 'NF && $1 !~ /^#/ { count++ } END { print count + 0 }' "$manifest")
unique_paths=$(awk 'NF && $1 !~ /^#/ { print }' "$manifest" | LC_ALL=C sort -u | wc -l | tr -d '[:space:]')
if [[ "$actual_paths" != "$expected_paths" || "$unique_paths" != "$expected_paths" ]]; then
    echo "error: $cohort object assignment manifest cardinality drifted" >&2
    exit 1
fi
awk 'NF && $1 !~ /^#/ { print }' "$manifest" | LC_ALL=C sort -c
actual_manifest=$(awk 'NF && $1 !~ /^#/ { print }' "$manifest" | sha256_stream)
if [[ "$actual_manifest" != "$expected_manifest" ]]; then
    echo "error: $cohort object assignment manifest content drifted" >&2
    exit 1
fi
if [[ "$(sha256_file "$manifest")" != "$expected_manifest_file" ]]; then
    echo "error: $cohort object assignment manifest file drifted" >&2
    exit 1
fi
if [[ "$(sha256_file "$profile")" != "$expected_profile" ]]; then
    echo "error: $cohort object assignment scoped capability profile drifted" >&2
    exit 1
fi
diff -u \
    <(awk 'NF && $1 !~ /^#/ { print }' "$manifest") \
    <(derive_manifest)
diff -u \
    <(profile_section audited-negative-tests | LC_ALL=C sort) \
    <(derive_negative_tests | LC_ALL=C sort)
while IFS= read -r test_path; do
    if [[ ! -f "$suite/$test_path" ]]; then
        echo "error: pinned $cohort object assignment path is missing: $test_path" >&2
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
    echo "error: $cohort object assignment report metadata drifted" >&2
    exit 1
fi

# Every admitted feature must be exercised, and every exercised feature must be
# admitted. This keeps each scoped profile narrower than the global frontier.
diff -u \
    <(profile_section features | LC_ALL=C sort) \
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
    echo "error: $cohort object assignment path/variant key set drifted" >&2
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
    echo "error: $cohort object assignment Test262 classified vector drifted" >&2
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

printf '%s object assignment Test262 gate passes: %s/%s variants across %s paths\n' \
    "$cohort" "$expected_passes" "$expected_variants" "$expected_paths"
