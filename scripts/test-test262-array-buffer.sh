#!/usr/bin/env bash
# Reproduce the checksum-bound pure ArrayBuffer Test262 core gate.

set -euo pipefail
export TZ=America/Los_Angeles

script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
root=$(CDPATH= cd -- "$script_dir/.." && pwd)
baseline=tests/test262-array-buffer-baseline.txt
manifest=tests/test262-array-buffer.txt
profile=tests/test262-array-buffer.conf
exclusions=tests/test262-array-buffer-exclusions.tsv
report=target/test262-array-buffer.tsv
json_report=target/test262-array-buffer.jsonl
oracle_log=target/test262-array-buffer-quickjs.log
workers=${TEST262_WORKERS:-8}

sha256_file() {
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum "$1" | awk '{ print $1 }'
    elif command -v shasum >/dev/null 2>&1; then
        shasum -a 256 "$1" | awk '{ print $1 }'
    else
        echo "error: sha256sum or shasum is required" >&2
        exit 2
    fi
}

sha256_stream() {
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum | awk '{ print $1 }'
    else
        shasum -a 256 | awk '{ print $1 }'
    fi
}

read_value() {
    local key=$1 value
    if ! value=$(awk -F= -v key="$key" '
        $1 == key { sub(/^[^=]*=/, ""); print; found++ }
        END { if (found != 1) exit 1 }
    ' "$baseline"); then
        echo "error: ArrayBuffer baseline is missing exactly one $key entry: $baseline" >&2
        exit 1
    fi
    if [[ -z "$value" ]]; then
        echo "error: ArrayBuffer baseline contains an empty $key entry: $baseline" >&2
        exit 1
    fi
    printf '%s\n' "$value"
}

read_header() {
    local key=$1
    awk -F= -v key="# $key" '
        $1 == key { sub(/^[^=]*=/, ""); print; found=1 }
        END { if (!found) exit 1 }
    ' "$report"
}

manifest_paths() {
    awk 'NF && $1 !~ /^#/ { print }' "$manifest"
}

exclusion_paths() {
    awk -F'\t' 'NF && $1 !~ /^#/ { print $1 }' "$exclusions"
}

profile_section() {
    local section=$1
    awk -v section="[$section]" '
        $0 == section { inside=1; next }
        /^\[/ { inside=0 }
        inside && NF && $1 !~ /^#/ { print }
    ' "$profile"
}

metadata_block() {
    local test_path=$1
    sed -n '/^\/\*---$/,/^---\*\/$/p' "$suite/$test_path"
}

metadata_list() {
    local test_path=$1 key=$2
    metadata_block "$test_path" | awk -v key="$key" '
        $0 ~ ("^" key ":[[:space:]]*\\[") {
            line=$0
            sub("^[^:]+:[[:space:]]*\\[", "", line)
            sub("\\][[:space:]]*$", "", line)
            count=split(line, values, /,[[:space:]]*/)
            for (i=1; i <= count; i++) {
                if (values[i] != "") print values[i]
            }
            exit
        }
        $0 == key ":" { inside=1; next }
        inside && /^[[:space:]]*-[[:space:]]*/ {
            line=$0
            sub(/^[[:space:]]*-[[:space:]]*/, "", line)
            if (line != "") print line
            next
        }
        inside { exit }
    '
}

verify_quickjs_oracle() {
    local runner=$source_dir/run-test262 test_path
    local -a files=()
    [[ -x "$runner" ]] || "${MAKE:-make}" -C "$source_dir" run-test262 >&2
    while IFS= read -r test_path; do
        files+=("test262/$test_path")
    done < <(manifest_paths)

    if ! (
        cd -- "$source_dir"
        # The QuickJS config selects its default mode, so `-a` must follow
        # `-c` to make sloppy/strict coverage explicit.
        ./run-test262 -m -c test262.conf -a -T "$workers" -f "${files[@]}"
    ) >"$oracle_log" 2>&1; then
        tail -n 100 "$oracle_log" >&2
        echo "error: pinned QuickJS could not execute the ArrayBuffer cohort" >&2
        exit 1
    fi
    if grep -Eq '(^|[[:space:]])FAILED($|[[:space:]])' "$oracle_log" \
        || ! grep -Fq \
            "Average memory statistics for $(read_value quickjs_variants) tests:" \
            "$oracle_log"; then
        tail -n 100 "$oracle_log" >&2
        echo "error: pinned QuickJS no longer passes all ArrayBuffer variants" >&2
        exit 1
    fi
}

cd -- "$root"

if [[ ! -f "$baseline" ]]; then
    echo "error: ArrayBuffer Test262 baseline is missing: $baseline" >&2
    echo "error: create the measured checksum baseline before running this gate" >&2
    exit 1
fi
for required in "$manifest" "$profile" "$exclusions"; do
    if [[ ! -f "$required" ]]; then
        echo "error: ArrayBuffer Test262 gate input is missing: $required" >&2
        exit 1
    fi
done
if [[ ! "$workers" =~ ^[1-9][0-9]*$ ]]; then
    echo "error: TEST262_WORKERS must be a positive integer, found: $workers" >&2
    exit 2
fi

expected_quickjs=$(read_value quickjs)
expected_test262=$(read_value test262)
expected_patch=$(read_value test262_patch_sha256)
expected_config=$(read_value test262_config_sha256)
expected_metadata=$(read_value test262_metadata_sha256)
expected_profile=$(read_value oxide_profile_sha256)
expected_schema=$(read_value schema)
expected_mode=$(read_value mode)
expected_timeout_ms=$(read_value timeout_ms)
expected_candidate_paths=$(read_value candidate_paths)
expected_candidate=$(read_value candidate_sha256)
expected_excluded_paths=$(read_value excluded_paths)
expected_exclusions=$(read_value exclusions_sha256)
expected_exclusions_file=$(read_value exclusions_file_sha256)
expected_paths=$(read_value paths)
expected_variants=$(read_value variants)
expected_quickjs_variants=$(read_value quickjs_variants)
expected_features=$(read_value features)
expected_features_hash=$(read_value features_sha256)
expected_includes=$(read_value includes)
expected_includes_hash=$(read_value includes_sha256)
expected_manifest=$(read_value manifest_sha256)
expected_manifest_file=$(read_value manifest_file_sha256)
expected_keys=$(read_value keys_sha256)
expected_runnable=$(read_value runnable)
expected_passes=$(read_value passes)
expected_failures=$(read_value failures)
expected_unsupported=$(read_value unsupported)
expected_skipped=$(read_value skipped)
expected_nonpass=$(read_value nonpass_sha256)
expected_tsv=$(read_value tsv_sha256)
expected_jsonl=$(read_value jsonl_sha256)
expected_summary=$(read_value summary)

if [[ "$expected_quickjs" != "2026-06-04" \
    || "$expected_test262" != "5c8206929d81b2d3d727ca6aac56c18358c8d790" \
    || "$expected_patch" != "f4b23b04641d438df0826fb17d7a5db276af2bdb085b42cc09aa8d50e0da9ba3" \
    || "$expected_config" != "79c64748ff1182baf5433d0a8378e3666738a785d02faf71f0d459ed42ae897b" \
    || "$expected_metadata" != "a37219960819e56a5c5c1723d31d6a33095c778bf5347385187fde96f927a06a" \
    || "$expected_profile" != "0803a027b2e9c238f80189993968816adfdda983ef3b23114a06f07b26c2d598" \
    || "$expected_schema" != "test262-canonical-classified-v2" \
    || "$expected_mode" != "both" \
    || "$expected_timeout_ms" != "30000" \
    || "$expected_candidate_paths" != "168" \
    || "$expected_candidate" != "5f946d35ef13229f710488a35a6b6e380c3bc2547f3af2a13da8c36cc4f701b9" \
    || "$expected_excluded_paths" != "24" \
    || "$expected_exclusions" != "5118e3de12f8d432856c99112ff9ec093da3e83f40c52a8c19c3b39b3d05b610" \
    || "$expected_exclusions_file" != "2f30dedf90aad3f0e6980c8f477dcffbbcacf8e6395666afd8b4b64dca95b51f" \
    || "$expected_paths" != "144" \
    || "$expected_variants" != "288" \
    || "$expected_quickjs_variants" != "288" \
    || "$expected_features" != "10" \
    || "$expected_features_hash" != "695d9a1cb17fbe72978bfbd6d870b61e823f732b5bf2410fab35b5933a309e57" \
    || "$expected_includes" != "3" \
    || "$expected_includes_hash" != "bf37abec464b3f2c2af165b0cc840a6e0b4ac6bef8e65d5115bd384bff731afd" \
    || "$expected_manifest" != "d5720cc22c785d3757eb4e30aa3de53a664d58133a2323c6afe6233788014d01" \
    || "$expected_manifest_file" != "d5720cc22c785d3757eb4e30aa3de53a664d58133a2323c6afe6233788014d01" \
    || "$expected_runnable" != "288" \
    || "$expected_passes" != "288" \
    || "$expected_failures" != "0" \
    || "$expected_unsupported" != "0" \
    || "$expected_skipped" != "0" \
    || "$expected_summary" != "pass=288" ]]; then
    echo "error: ArrayBuffer baseline identity drifted" >&2
    exit 1
fi

if [[ "$expected_keys" == "PENDING" \
    || "$expected_nonpass" == "PENDING" \
    || "$expected_tsv" == "PENDING" \
    || "$expected_jsonl" == "PENDING" ]]; then
    echo "error: ArrayBuffer Test262 baseline still contains PENDING measured hashes" >&2
    exit 1
fi

suite=$("$script_dir/prepare-test262.sh")
source_dir=$(dirname -- "$suite")
if [[ "$(basename -- "$source_dir")" != "quickjs-$expected_quickjs" \
    || "$(git -C "$suite" rev-parse --verify 'HEAD^{commit}')" != "$expected_test262" \
    || "$(sha256_file "$source_dir/tests/test262.patch")" != "$expected_patch" \
    || "$(sha256_file "$source_dir/test262.conf")" != "$expected_config" ]]; then
    echo "error: prepared QuickJS/Test262 inputs drifted from the ArrayBuffer baseline" >&2
    exit 1
fi

tmp_dir=$(mktemp -d "${TMPDIR:-/tmp}/quickjs-oxide-array-buffer.XXXXXX")
trap 'rm -rf -- "$tmp_dir"' EXIT HUP INT TERM
manifest_inventory=$tmp_dir/manifest.txt
excluded_inventory=$tmp_dir/excluded.txt
candidate_inventory=$tmp_dir/candidate.txt
derived_exclusions=$tmp_dir/derived-exclusions.txt
derived_manifest=$tmp_dir/derived-manifest.txt
feature_occurrences=$tmp_dir/features.raw
include_occurrences=$tmp_dir/includes.raw
feature_inventory=$tmp_dir/features.txt
include_inventory=$tmp_dir/includes.txt
variant_keys=$tmp_dir/variant-keys.txt

manifest_paths >"$manifest_inventory"
exclusion_paths >"$excluded_inventory"
LC_ALL=C sort -c "$manifest_inventory"
LC_ALL=C sort -c "$excluded_inventory"

actual_paths=$(wc -l <"$manifest_inventory" | tr -d '[:space:]')
unique_paths=$(LC_ALL=C sort -u "$manifest_inventory" | wc -l | tr -d '[:space:]')
if [[ "$actual_paths" != "$expected_paths" || "$unique_paths" != "$expected_paths" \
    || "$(sha256_stream <"$manifest_inventory")" != "$expected_manifest" \
    || "$(sha256_file "$manifest")" != "$expected_manifest_file" ]]; then
    echo "error: ArrayBuffer manifest cardinality or content drifted" >&2
    exit 1
fi
if [[ "$(sha256_file "$profile")" != "$expected_profile" ]]; then
    echo "error: ArrayBuffer scoped capability profile drifted" >&2
    exit 1
fi

if ! awk -F'\t' '
    NR == 1 {
        if ($1 != "# path" || $2 != "reason" || NF != 2) exit 1
        next
    }
    {
        if (NF != 2 || $1 == "" \
            || $2 != "source directly uses Uint8Array without declaring TypedArray metadata") {
            exit 1
        }
    }
    END { if (NR != 25) exit 1 }
' "$exclusions"; then
    echo "error: ArrayBuffer latent Uint8Array exclusion ledger format drifted" >&2
    exit 1
fi
if [[ "$(wc -l <"$excluded_inventory" | tr -d '[:space:]')" != "$expected_excluded_paths" \
    || "$(LC_ALL=C sort -u "$excluded_inventory" | wc -l | tr -d '[:space:]')" \
        != "$expected_excluded_paths" \
    || "$(sha256_stream <"$excluded_inventory")" != "$expected_exclusions" \
    || "$(sha256_file "$exclusions")" != "$expected_exclusions_file" ]]; then
    echo "error: ArrayBuffer latent Uint8Array exclusion inventory drifted" >&2
    exit 1
fi
if [[ -n "$(comm -12 "$manifest_inventory" "$excluded_inventory")" ]]; then
    echo "error: ArrayBuffer manifest overlaps its latent Uint8Array exclusions" >&2
    exit 1
fi

LC_ALL=C sort -u "$manifest_inventory" "$excluded_inventory" >"$candidate_inventory"
if [[ "$(wc -l <"$candidate_inventory" | tr -d '[:space:]')" != "$expected_candidate_paths" \
    || "$(sha256_file "$candidate_inventory")" != "$expected_candidate" ]]; then
    echo "error: ArrayBuffer 168-path pre-Uint8Array candidate inventory drifted" >&2
    exit 1
fi

: >"$derived_exclusions"
while IFS= read -r test_path; do
    if [[ "$test_path" != test/built-ins/ArrayBuffer/* || ! -f "$suite/$test_path" ]]; then
        echo "error: invalid or missing ArrayBuffer candidate path: $test_path" >&2
        exit 1
    fi
    if grep -Eq '(^|[^[:alnum:]_$])Uint8Array([^[:alnum:]_$]|$)' "$suite/$test_path"; then
        printf '%s\n' "$test_path" >>"$derived_exclusions"
    fi
done <"$candidate_inventory"
LC_ALL=C sort -o "$derived_exclusions" "$derived_exclusions"
diff -u "$excluded_inventory" "$derived_exclusions"

while IFS= read -r test_path; do
    if metadata_list "$test_path" features | grep -Fxq TypedArray; then
        echo "error: latent Uint8Array exclusion now declares TypedArray: $test_path" >&2
        exit 1
    fi
done <"$excluded_inventory"

comm -23 "$candidate_inventory" "$excluded_inventory" >"$derived_manifest"
diff -u "$derived_manifest" "$manifest_inventory"

: >"$feature_occurrences"
: >"$include_occurrences"
: >"$variant_keys"
while IFS= read -r test_path; do
    metadata=$(metadata_block "$test_path")
    if [[ -z "$metadata" ]]; then
        echo "error: ArrayBuffer path lost Test262 metadata: $test_path" >&2
        exit 1
    fi
    if grep -Fq 'negative:' <<<"$metadata"; then
        echo "error: ArrayBuffer all-green cohort gained a negative test: $test_path" >&2
        exit 1
    fi
    if [[ -n "$(metadata_list "$test_path" flags)" ]]; then
        echo "error: ArrayBuffer cohort gained non-default variant flags: $test_path" >&2
        exit 1
    fi
    metadata_list "$test_path" features >>"$feature_occurrences"
    metadata_list "$test_path" includes >>"$include_occurrences"
    printf '%s\tsloppy\n%s\tstrict\n' "$test_path" "$test_path" >>"$variant_keys"
done <"$manifest_inventory"

LC_ALL=C sort -u "$feature_occurrences" >"$feature_inventory"
LC_ALL=C sort -u "$include_occurrences" >"$include_inventory"
LC_ALL=C sort -o "$variant_keys" "$variant_keys"
if [[ "$(wc -l <"$feature_inventory" | tr -d '[:space:]')" != "$expected_features" \
    || "$(sha256_file "$feature_inventory")" != "$expected_features_hash" ]]; then
    echo "error: ArrayBuffer Test262 feature stream drifted" >&2
    exit 1
fi
if [[ "$(wc -l <"$include_inventory" | tr -d '[:space:]')" != "$expected_includes" \
    || "$(sha256_file "$include_inventory")" != "$expected_includes_hash" ]]; then
    echo "error: ArrayBuffer Test262 include stream drifted" >&2
    exit 1
fi
if [[ "$(wc -l <"$variant_keys" | tr -d '[:space:]')" != "$expected_variants" \
    || "$(sha256_file "$variant_keys")" != "$expected_keys" ]]; then
    echo "error: ArrayBuffer Test262 path/variant key stream drifted" >&2
    exit 1
fi
diff -u <(profile_section features | LC_ALL=C sort) "$feature_inventory"
if [[ -n "$(profile_section audited-negative-tests)" \
    || -n "$(profile_section execution)" ]]; then
    echo "error: ArrayBuffer scoped profile must contain neither negatives nor execution opt-ins" >&2
    exit 1
fi

verify_quickjs_oracle

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

actual_variants=$(awk -F'\t' \
    '!/^#/ && !($1 == "path" && $2 == "variant") { count++ } END { print count + 0 }' \
    "$report")
execution_line=$(printf '%s\n' "$run_output" \
    | awk '/^execution: runnable=/ { print; found=1 } END { if (!found) exit 1 }')
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
    echo "error: ArrayBuffer Test262 report metadata drifted" >&2
    exit 1
fi

diff -u \
    "$manifest_inventory" \
    <(awk -F'\t' \
        '!/^#/ && !($1 == "path" && $2 == "variant") { print $1 }' \
        "$report" | LC_ALL=C sort -u)
diff -u \
    "$feature_inventory" \
    <(awk -F'\t' '
        !/^#/ && !($1 == "path" && $2 == "variant") {
            count=split($4, features, ",")
            for (i=1; i <= count; i++) {
                if (features[i] != "") print features[i]
            }
        }
    ' "$report" | LC_ALL=C sort -u)

actual_keys=$(awk -F'\t' \
    '!/^#/ && !($1 == "path" && $2 == "variant") { print $1 "\t" $2 }' \
    "$report" | LC_ALL=C sort | sha256_stream)
actual_passes=$(awk -F'\t' \
    '!/^#/ && !($1 == "path" && $2 == "variant") && $7 == "pass" { count++ }
    END { print count + 0 }' "$report")
actual_unsupported=$(awk -F'\t' \
    '!/^#/ && !($1 == "path" && $2 == "variant") && $7 ~ /^unsupported-/ { count++ }
    END { print count + 0 }' "$report")
actual_skipped=$(awk -F'\t' \
    '!/^#/ && !($1 == "path" && $2 == "variant") && $7 ~ /^skipped-/ { count++ }
    END { print count + 0 }' "$report")
actual_failures=$((actual_variants - actual_passes - actual_unsupported - actual_skipped))
actual_nonpass=$(awk -F'\t' \
    '!/^#/ && !($1 == "path" && $2 == "variant") && $7 != "pass" {
        print $1 "\t" $2 "\t" $7 "\t" $8 "\t" $9 "\t" $10
    }' "$report" | sha256_stream)
actual_summary=$(tail -n 1 "$report" | sed 's/^# summary //')
runner_summary=$(printf '%s\n' "$run_output" \
    | awk '/^Test262: total=/ { print; found=1 } END { if (!found) exit 1 }')
expected_runner_summary="Test262: total=$expected_variants pass=$expected_passes fail=$expected_failures unsupported=$expected_unsupported skipped=$expected_skipped"

if [[ "$runner_summary" != "$expected_runner_summary" \
    || "$actual_passes" != "$expected_passes" \
    || "$actual_failures" != "$expected_failures" \
    || "$actual_unsupported" != "$expected_unsupported" \
    || "$actual_skipped" != "$expected_skipped" \
    || "$actual_keys" != "$expected_keys" \
    || "$actual_nonpass" != "$expected_nonpass" \
    || "$actual_summary" != "$expected_summary" \
    || "$(sha256_file "$report")" != "$expected_tsv" \
    || "$(sha256_file "$json_report")" != "$expected_jsonl" ]]; then
    echo "error: ArrayBuffer Test262 classified vector drifted" >&2
    printf 'path\tvariant\toutcome\tactual_phase\tactual_type\tdetail\n' >&2
    awk -F'\t' '
        !/^#/ && !($1 == "path" && $2 == "variant") && $7 != "pass" {
            print $1 "\t" $2 "\t" $7 "\t" $8 "\t" $9 "\t" $10
            if (++shown == 80) exit
        }
    ' "$report" >&2
    exit 1
fi

printf 'ArrayBuffer Test262 gate passes: %s/%s variants across %s paths; pinned QuickJS passes %s/%s\n' \
    "$expected_passes" \
    "$expected_variants" \
    "$expected_paths" \
    "$expected_quickjs_variants" \
    "$expected_quickjs_variants"
