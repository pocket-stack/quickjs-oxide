#!/usr/bin/env bash
# Reproduce the dependency-audited Iterator.concat sequencing Test262 gate.

set -euo pipefail
export TZ=America/Los_Angeles

script_dir=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd)
root=$(CDPATH='' cd -- "$script_dir/.." && pwd)
baseline=tests/test262-iterator-sequencing-baseline.txt
manifest=tests/test262-iterator-sequencing.txt
admission_profile=tests/test262-iterator-sequencing.conf
global_profile=compat/test262-oxide.conf
report=target/test262-iterator-sequencing.tsv
json_report=target/test262-iterator-sequencing.jsonl
workers=${TEST262_WORKERS:-8}

usage() {
    printf 'usage: %s [--check|--bless]\n' "${0##*/}"
    printf '  --check  verify the frozen inventory/profile and both pinned QuickJS modes only\n'
    printf '  --bless  record a new Oxide baseline, but only when every frozen variant passes\n'
}

check_only=false
bless=false
case ${1-} in
    "") ;;
    --check) check_only=true ;;
    --bless) bless=true ;;
    -h | --help) usage; exit 0 ;;
    *) usage >&2; exit 2 ;;
esac
[[ $# -le 1 ]] || { usage >&2; exit 2; }

read_value() {
    local key=$1
    awk -F= -v key="$key" '
        $1 == key { sub(/^[^=]*=/, ""); print; found=1 }
        END { if (!found) exit 1 }
    ' "$baseline"
}

read_header() {
    local key=$1
    awk -F= -v key="# $key" '
        $1 == key { sub(/^[^=]*=/, ""); print; found=1 }
        END { if (!found) exit 1 }
    ' "$report"
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

manifest_paths() {
    awk 'NF && $1 !~ /^#/ { print }' "$manifest"
}

profile_section() {
    local profile=$1 section=$2
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
            sub("^[^:]+:[[:space:]]*\\[", "")
            sub(/\][[:space:]]*$/, "")
            count=split($0, values, /,[[:space:]]*/)
            for (i=1; i<=count; i++) if (values[i] != "") print values[i]
            exit
        }
        $0 ~ ("^" key ":[[:space:]]*$") { inside=1; next }
        inside && /^[[:space:]]+-[[:space:]]+/ {
            sub(/^[[:space:]]+-[[:space:]]+/, "")
            print
            next
        }
        inside { exit }
    '
}

program_body() {
    local test_path=$1
    sed '/^\/\*---$/,/^---\*\/$/d' "$suite/$test_path"
}

verify_inventory() {
    local name=$1 inventory=$2 expected_count expected_hash actual_count actual_hash
    expected_count=$(read_value "${name}_paths")
    expected_hash=$(read_value "${name}_sha256")
    actual_count=$(printf '%s\n' "$inventory" | sed '/^$/d' | wc -l | tr -d '[:space:]')
    actual_hash=$(printf '%s\n' "$inventory" | sed '/^$/d' | sha256_stream)
    if [[ "$actual_count" != "$expected_count" || "$actual_hash" != "$expected_hash" ]]; then
        echo "error: Iterator sequencing $name inventory drifted" >&2
        exit 1
    fi
}

verify_quickjs_oracle() {
    local runner=$source_dir/run-test262 sloppy_output strict_output test_path
    local -a files=()
    [[ -x "$runner" ]] || "${MAKE:-make}" -C "$source_dir" run-test262 >&2
    while IFS= read -r test_path; do
        files+=("test262/$test_path")
    done < <(manifest_paths)

    if ! sloppy_output=$(cd -- "$source_dir" \
        && ./run-test262 -m -c test262.conf -f "${files[@]}" 2>&1); then
        printf '%s\n' "$sloppy_output" >&2
        echo "error: pinned QuickJS could not execute the sloppy Iterator sequencing cohort" >&2
        exit 1
    fi
    if ! strict_output=$(cd -- "$source_dir" \
        && ./run-test262 -s -m -c test262.conf -f "${files[@]}" 2>&1); then
        printf '%s\n' "$strict_output" >&2
        echo "error: pinned QuickJS could not execute the strict Iterator sequencing cohort" >&2
        exit 1
    fi

    if grep -Eq '(^|[[:space:]])FAILED($|[[:space:]])' <<<"$sloppy_output" \
        || ! grep -Fq "Average memory statistics for $(read_value quickjs_sloppy_passes) tests:" \
            <<<"$sloppy_output"; then
        printf '%s\n' "$sloppy_output" >&2
        echo "error: pinned QuickJS no longer passes every sloppy Iterator sequencing path" >&2
        exit 1
    fi
    if grep -Eq '(^|[[:space:]])FAILED($|[[:space:]])' <<<"$strict_output" \
        || ! grep -Fq "Average memory statistics for $(read_value quickjs_strict_passes) tests:" \
            <<<"$strict_output"; then
        printf '%s\n' "$strict_output" >&2
        echo "error: pinned QuickJS no longer passes every strict Iterator sequencing path" >&2
        exit 1
    fi
}

cd -- "$root"
suite=$("$script_dir/prepare-test262.sh")
source_dir=$(dirname -- "$suite")

if [[ "$(read_value quickjs)" != "2026-06-04" \
    || "$(read_value test262)" != "5c8206929d81b2d3d727ca6aac56c18358c8d790" \
    || "$(read_value test262_patch_sha256)" != "f4b23b04641d438df0826fb17d7a5db276af2bdb085b42cc09aa8d50e0da9ba3" \
    || "$(read_value test262_config_sha256)" != "79c64748ff1182baf5433d0a8378e3666738a785d02faf71f0d459ed42ae897b" \
    || "$(read_value test262_metadata_sha256)" != "a37219960819e56a5c5c1723d31d6a33095c778bf5347385187fde96f927a06a" \
    || "$(read_value global_oxide_profile_sha256)" != "d01f4f49fbd14b2cad610983624142b468587b2e0bd10ae6264641c39cffa05f" \
    || "$(read_value oxide_profile_sha256)" != "ee7e5626b6c27a9f4a8257984439ca2641d31258521e060fce24101cf1d1e0f0" \
    || "$(read_value profile_features)" != "74" \
    || "$(read_value profile_features_sha256)" != "35a65d9d6522e84627168c456a49a387561c7e087d51c616159aad304373006e" \
    || "$(read_value schema)" != "test262-canonical-classified-v2" \
    || "$(read_value mode)" != "both" \
    || "$(read_value timeout_ms)" != "30000" \
    || "$(read_value tagged_paths)" != "32" \
    || "$(read_value proxy_paths)" != "0" \
    || "$(read_value harness_proxy_paths)" != "0" \
    || "$(read_value create_realm_paths)" != "0" \
    || "$(read_value is_html_dda_paths)" != "0" \
    || "$(read_value config_excluded_paths)" != "0" \
    || "$(read_value excluded_paths)" != "0" \
    || "$(read_value paths)" != "32" \
    || "$(read_value variants)" != "64" \
    || "$(read_value quickjs_sloppy_passes)" != "32" \
    || "$(read_value quickjs_strict_passes)" != "32" \
    || "$(read_value quickjs_passes)" != "64" \
    || "$(read_value features)" != "1" \
    || "$(read_value includes)" != "2" ]]; then
    echo "error: Iterator sequencing baseline identity drifted" >&2
    exit 1
fi

tagged_inventory=$(
    git -C "$suite" grep -l -F 'iterator-sequencing' -- 'test/**/*.js' \
        | while IFS= read -r test_path; do
            if metadata_list "$test_path" features | grep -Fxq 'iterator-sequencing'; then
                printf '%s\n' "$test_path"
            fi
        done \
        | LC_ALL=C sort
)
verify_inventory tagged "$tagged_inventory"

proxy_inventory=$(
    while IFS= read -r test_path; do
        if program_body "$test_path" \
            | grep -Eq '(^|[^[:alnum:]_$])Proxy([^[:alnum:]_$]|$)'; then
            printf '%s\n' "$test_path"
        fi
    done <<<"$tagged_inventory"
)
harness_proxy_inventory=$(
    while IFS= read -r test_path; do
        while IFS= read -r include; do
            if [[ -f "$suite/harness/$include" ]] \
                && grep -Eq '(^|[^[:alnum:]_$])Proxy([^[:alnum:]_$]|$)' \
                    "$suite/harness/$include"; then
                printf '%s\n' "$test_path"
                break
            fi
        done < <(metadata_list "$test_path" includes)
    done <<<"$tagged_inventory"
)
create_realm_inventory=$(
    while IFS= read -r test_path; do
        if program_body "$test_path" | grep -Eq '[$]262[.]createRealm([^[:alnum:]_$]|$)'; then
            printf '%s\n' "$test_path"
        fi
    done <<<"$tagged_inventory"
)
is_html_dda_inventory=$(
    while IFS= read -r test_path; do
        if program_body "$test_path" | grep -Eq '[$]262[.]IsHTMLDDA([^[:alnum:]_$]|$)'; then
            printf '%s\n' "$test_path"
        fi
    done <<<"$tagged_inventory"
)
config_excluded_inventory=$(
    while IFS= read -r test_path; do
        if awk '
            $0 == "[exclude]" { inside=1; next }
            /^\[/ { inside=0 }
            inside && NF && $1 !~ /^#/ { print }
        ' "$source_dir/test262.conf" | grep -Fxq "test262/$test_path"; then
            printf '%s\n' "$test_path"
        fi
    done <<<"$tagged_inventory"
)

verify_inventory proxy "$proxy_inventory"
verify_inventory harness_proxy "$harness_proxy_inventory"
verify_inventory create_realm "$create_realm_inventory"
verify_inventory is_html_dda "$is_html_dda_inventory"
verify_inventory config_excluded "$config_excluded_inventory"

excluded_inventory=$(
    printf '%s\n%s\n%s\n%s\n%s\n' \
        "$proxy_inventory" \
        "$harness_proxy_inventory" \
        "$create_realm_inventory" \
        "$is_html_dda_inventory" \
        "$config_excluded_inventory" \
        | sed '/^$/d' \
        | LC_ALL=C sort -u
)
verify_inventory excluded "$excluded_inventory"

derived_manifest=$(
    comm -23 \
        <(printf '%s\n' "$tagged_inventory") \
        <(printf '%s\n' "$excluded_inventory")
)
diff -u <(printf '%s\n' "$derived_manifest") <(manifest_paths)

actual_paths=$(manifest_paths | wc -l | tr -d '[:space:]')
unique_paths=$(manifest_paths | LC_ALL=C sort -u | wc -l | tr -d '[:space:]')
[[ "$actual_paths" == "32" && "$unique_paths" == "32" ]] \
    || { echo "error: Iterator sequencing manifest cardinality drifted" >&2; exit 1; }
manifest_paths | LC_ALL=C sort -c
[[ "$(manifest_paths | sha256_stream)" == "$(read_value manifest_sha256)" \
    && "$(sha256_file "$manifest")" == "$(read_value manifest_file_sha256)" ]] \
    || { echo "error: Iterator sequencing manifest content drifted" >&2; exit 1; }

feature_inventory=
include_inventory=
variant_keys=
while IFS= read -r test_path; do
    [[ -f "$suite/$test_path" ]] \
        || { echo "error: missing Iterator sequencing path: $test_path" >&2; exit 1; }
    metadata=$(metadata_block "$test_path")
    if grep -Fq 'negative:' <<<"$metadata"; then
        echo "error: Iterator sequencing cohort unexpectedly gained a negative test: $test_path" >&2
        exit 1
    fi
    flag_line=$(grep '^flags:' <<<"$metadata" || true)
    case "$flag_line" in
        "" | "flags: []") ;;
        *)
            echo "error: Iterator sequencing path no longer has both ordinary variants: $test_path: $flag_line" >&2
            exit 1
            ;;
    esac
    features=$(metadata_list "$test_path" features)
    grep -Fxq 'iterator-sequencing' <<<"$features" \
        || { echo "error: selected path lost iterator-sequencing metadata: $test_path" >&2; exit 1; }
    feature_inventory+=$'\n'"$features"
    include_inventory+=$'\n'"$(metadata_list "$test_path" includes)"
    variant_keys+=$'\n'"$test_path"$'\t'sloppy$'\n'"$test_path"$'\t'strict
done < <(manifest_paths)

feature_inventory=$(printf '%s\n' "$feature_inventory" | sed '/^$/d' | LC_ALL=C sort -u)
include_inventory=$(printf '%s\n' "$include_inventory" | sed '/^$/d' | LC_ALL=C sort -u)
variant_keys=$(printf '%s\n' "$variant_keys" | sed '/^$/d' | LC_ALL=C sort)

[[ "$(printf '%s\n' "$feature_inventory" | wc -l | tr -d '[:space:]')" == "$(read_value features)" \
    && "$(printf '%s\n' "$feature_inventory" | sha256_stream)" == "$(read_value features_sha256)" ]] \
    || { echo "error: Iterator sequencing metadata features drifted" >&2; exit 1; }
[[ "$(printf '%s\n' "$include_inventory" | wc -l | tr -d '[:space:]')" == "$(read_value includes)" \
    && "$(printf '%s\n' "$include_inventory" | sha256_stream)" == "$(read_value includes_sha256)" ]] \
    || { echo "error: Iterator sequencing harness includes drifted" >&2; exit 1; }
[[ "$(printf '%s\n' "$variant_keys" | wc -l | tr -d '[:space:]')" == "$(read_value variants)" \
    && "$(printf '%s\n' "$variant_keys" | sha256_stream)" == "$(read_value keys_sha256)" ]] \
    || { echo "error: Iterator sequencing sloppy/strict key inventory drifted" >&2; exit 1; }

global_features=$(profile_section "$global_profile" features | LC_ALL=C sort)
admission_features=$(profile_section "$admission_profile" features | LC_ALL=C sort)
profile_section "$global_profile" features | LC_ALL=C sort -c
profile_section "$admission_profile" features | LC_ALL=C sort -c
[[ -z "$(comm -23 <(printf '%s\n' "$global_features") <(printf '%s\n' "$admission_features"))" ]] \
    || { echo "error: Iterator sequencing profile removed a global capability" >&2; exit 1; }
diff -u \
    <(printf '%s\n' iterator-sequencing) \
    <(comm -13 <(printf '%s\n' "$global_features") <(printf '%s\n' "$admission_features"))
[[ "$(printf '%s\n' "$admission_features" | wc -l | tr -d '[:space:]')" == "$(read_value profile_features)" \
    && "$(printf '%s\n' "$admission_features" | sha256_stream)" == "$(read_value profile_features_sha256)" ]] \
    || { echo "error: Iterator sequencing profile feature inventory drifted" >&2; exit 1; }
[[ -z "$(comm -23 <(printf '%s\n' "$feature_inventory") <(printf '%s\n' "$admission_features"))" ]] \
    || { echo "error: Iterator sequencing metadata exceeds the scoped profile" >&2; exit 1; }
diff -u \
    <(awk '$0 == "[audited-negative-tests]" { inside=1 } inside { print }' "$global_profile") \
    <(awk '$0 == "[audited-negative-tests]" { inside=1 } inside { print }' "$admission_profile")
[[ -z "$(profile_section "$admission_profile" execution)" ]] \
    || { echo "error: Iterator.concat sequencing profile admitted an execution capability" >&2; exit 1; }
[[ "$(sha256_file "$global_profile")" == "$(read_value global_oxide_profile_sha256)" \
    && "$(sha256_file "$admission_profile")" == "$(read_value oxide_profile_sha256)" ]] \
    || { echo "error: Iterator sequencing capability profile drifted" >&2; exit 1; }

verify_quickjs_oracle
if "$check_only"; then
    printf 'Iterator sequencing inputs verified: %s tagged - %s source Proxy - %s harness Proxy - %s host/config = %s clean paths; QuickJS %s passes %s/%s sloppy+strict variants\n' \
        "$(read_value tagged_paths)" \
        "$(read_value proxy_paths)" \
        "$(read_value harness_proxy_paths)" \
        "$(( $(read_value create_realm_paths) + $(read_value is_html_dda_paths) + $(read_value config_excluded_paths) ))" \
        "$(read_value paths)" \
        "$(read_value quickjs)" \
        "$(read_value quickjs_passes)" \
        "$(read_value variants)"
    exit 0
fi

pending_keys=()
for key in runnable passes failures unsupported skipped nonpass_sha256 tsv_sha256 jsonl_sha256 summary; do
    if [[ "$(read_value "$key")" == "PENDING" ]]; then
        pending_keys+=("$key")
    fi
done
if [[ ${#pending_keys[@]} -ne 0 ]] && ! "$bless"; then
    printf 'error: Iterator sequencing Oxide baseline needs refresh after implementation: %s\n' \
        "${pending_keys[*]}" >&2
    exit 1
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
expected_variants=$(read_value variants)
expected_runnable=$(read_value runnable)
expected_passes=$(read_value passes)
expected_failures=$(read_value failures)
expected_unsupported=$(read_value unsupported)
expected_skipped=$(read_value skipped)
expected_keys=$(read_value keys_sha256)
expected_nonpass=$(read_value nonpass_sha256)
expected_tsv=$(read_value tsv_sha256)
expected_jsonl=$(read_value jsonl_sha256)
expected_summary=$(read_value summary)

rm -f -- "$report" "$json_report"
run_output=$(cargo run --locked --release --quiet --bin run-test262 -- \
    --suite "$suite" \
    --config "$source_dir/test262.conf" \
    --oxide-profile "$admission_profile" \
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
required_runnable=$expected_runnable
if "$bless"; then
    required_runnable=$expected_variants
fi
if [[ "$(read_header quickjs)" != "$expected_quickjs" \
    || "$(read_header test262)" != "$expected_test262" \
    || "$(read_header test262_patch_sha256)" != "$expected_patch" \
    || "$(read_header test262_config_sha256)" != "$expected_config" \
    || "$(read_header test262_metadata_sha256)" != "$expected_metadata" \
    || "$(read_header oxide_profile_sha256)" != "$expected_profile" \
    || "$(read_header profile)" != "$expected_schema" \
    || "$(read_header mode)" != "$expected_mode" \
    || "$actual_variants" != "$expected_variants" \
    || "$actual_runnable" != "$required_runnable" ]]; then
    echo "error: Iterator sequencing report metadata drifted" >&2
    exit 1
fi

diff -u \
    <(printf '%s\n' "$feature_inventory") \
    <(awk -F'\t' '
        !/^#/ && !($1 == "path" && $2 == "variant") {
            count=split($4, features, ",")
            for (i=1; i<=count; i++) if (features[i] != "") print features[i]
        }
    ' "$report" | LC_ALL=C sort -u)
diff -u \
    <(manifest_paths | LC_ALL=C sort) \
    <(awk -F'\t' '!/^#/ && !($1 == "path" && $2 == "variant") { print $1 }' "$report" | LC_ALL=C sort -u)

actual_keys=$(awk -F'\t' '!/^#/ && !($1 == "path" && $2 == "variant") { print $1 "\t" $2 }' "$report" | LC_ALL=C sort | sha256_stream)
actual_passes=$(awk -F'\t' '!/^#/ && !($1 == "path" && $2 == "variant") && $7 == "pass" { count++ } END { print count + 0 }' "$report")
actual_unsupported=$(awk -F'\t' '!/^#/ && !($1 == "path" && $2 == "variant") && $7 ~ /^unsupported-/ { count++ } END { print count + 0 }' "$report")
actual_skipped=$(awk -F'\t' '!/^#/ && !($1 == "path" && $2 == "variant") && $7 ~ /^skipped-/ { count++ } END { print count + 0 }' "$report")
actual_failures=$((actual_variants - actual_passes - actual_unsupported - actual_skipped))
actual_nonpass=$(awk -F'\t' '!/^#/ && !($1 == "path" && $2 == "variant") && $7 != "pass" { print $1 "\t" $2 "\t" $7 "\t" $8 "\t" $9 "\t" $10 }' "$report" | sha256_stream)
actual_summary=$(tail -n 1 "$report" | sed 's/^# summary //')
runner_summary=$(printf '%s\n' "$run_output" | awk '/^Test262: total=/ { print; found=1 } END { if (!found) exit 1 }')

if "$bless"; then
    if [[ "$actual_passes" != "$expected_variants" \
        || "$actual_failures" != "0" \
        || "$actual_unsupported" != "0" \
        || "$actual_skipped" != "0" \
        || "$actual_keys" != "$expected_keys" \
        || "$runner_summary" != "Test262: total=$expected_variants pass=$expected_variants fail=0 unsupported=0 skipped=0" ]]; then
        echo "error: refusing to bless a non-green Iterator sequencing vector" >&2
        exit 1
    fi

    actual_tsv=$(sha256_file "$report")
    actual_jsonl=$(sha256_file "$json_report")
    baseline_tmp=$(mktemp "$baseline.XXXXXX")
    awk -F= \
        -v runnable="$actual_runnable" \
        -v passes="$actual_passes" \
        -v failures="$actual_failures" \
        -v unsupported="$actual_unsupported" \
        -v skipped="$actual_skipped" \
        -v nonpass_sha256="$actual_nonpass" \
        -v tsv_sha256="$actual_tsv" \
        -v jsonl_sha256="$actual_jsonl" \
        -v summary="$actual_summary" '
        BEGIN {
            replacement["runnable"] = runnable
            replacement["passes"] = passes
            replacement["failures"] = failures
            replacement["unsupported"] = unsupported
            replacement["skipped"] = skipped
            replacement["nonpass_sha256"] = nonpass_sha256
            replacement["tsv_sha256"] = tsv_sha256
            replacement["jsonl_sha256"] = jsonl_sha256
            replacement["summary"] = summary
        }
        $1 in replacement {
            print $1 "=" replacement[$1]
            next
        }
        { print }
    ' "$baseline" >"$baseline_tmp"
    chmod 644 "$baseline_tmp"
    mv -- "$baseline_tmp" "$baseline"
    printf 'Iterator sequencing baseline blessed from an all-green vector: %s/%s pass\n' \
        "$actual_passes" "$actual_variants"
    exit 0
fi

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
    echo "error: Iterator sequencing Test262 classified vector drifted" >&2
    awk -F'\t' '
        !/^#/ && !($1 == "path" && $2 == "variant") && $7 != "pass" {
            print $1 "\t" $2 "\t" $7 "\t" $8 "\t" $9 "\t" $10
        }
    ' "$report" >&2
    exit 1
fi

printf 'Iterator sequencing Test262 gate is exact: %s/%s pass across %s clean paths\n' \
    "$actual_passes" "$actual_variants" "$(read_value paths)"
