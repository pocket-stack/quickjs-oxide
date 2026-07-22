#!/usr/bin/env bash
# Reproduce the complete dependency-audited R3h private class-fields cohort.

set -euo pipefail
export TZ=America/Los_Angeles

script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
root=$(CDPATH= cd -- "$script_dir/.." && pwd)
baseline=tests/test262-class-private-fields-baseline.txt
manifest=tests/test262-class-private-fields.txt
admission_profile=tests/test262-class-private-fields.conf
global_profile=compat/test262-oxide.conf
report=target/test262-class-private-fields.tsv
json_report=target/test262-class-private-fields.jsonl
workers=${TEST262_WORKERS:-8}

usage() {
    printf 'usage: %s [--check]\n' "${0##*/}"
    printf '  --check  verify frozen inputs and the pinned QuickJS oracle only\n'
}

check_only=false
case ${1-} in
    "") ;;
    --check) check_only=true ;;
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

manifest_paths() {
    awk 'NF && $1 !~ /^#/ { print }' "$manifest"
}

tagged_paths() {
    git -C "$suite" grep -l -E \
        'class-fields-private|class-static-fields-private|class-fields-private-in' \
        -- test \
        | LC_ALL=C sort
}

profile_section() {
    local section=$1
    awk -v section="[$section]" '
        $0 == section { inside=1; next }
        /^\[/ { inside=0 }
        inside && NF && $1 !~ /^#/ { print }
    ' "$admission_profile"
}

metadata_block() {
    local test_path=$1
    sed -n '/^\/\*---$/,/^---\*\/$/p' "$suite/$test_path"
}

metadata_features() {
    local test_path=$1
    metadata_block "$test_path" | awk '
        /^features:[[:space:]]*\[/ {
            sub(/^features:[[:space:]]*\[/, "")
            sub(/\][[:space:]]*$/, "")
            count=split($0, values, /,[[:space:]]*/)
            for (i=1; i<=count; i++) if (values[i] != "") print values[i]
            done=1
            exit
        }
        /^features:[[:space:]]*$/ { inside=1; next }
        inside && /^[[:space:]]*-[[:space:]]*/ {
            sub(/^[[:space:]]*-[[:space:]]*/, "")
            print
            next
        }
        inside { done=1; exit }
    '
}

derive_candidate_paths() {
    local test_path metadata features
    while IFS= read -r test_path; do
        metadata=$(metadata_block "$test_path")
        features=$(metadata_features "$test_path")
        if ! grep -Eq '^(class-fields-private|class-static-fields-private|class-fields-private-in)$' \
            <<<"$features"; then
            continue
        fi
        if grep -Eq '^(class-methods-private|class-static-methods-private|async-functions|async-iteration|generators|destructuring-binding|logical-assignment-operators|optional-chaining|exponentiation|BigInt|Proxy|cross-realm|nonextensible-applies-to-private|class-static-block)$' \
            <<<"$features"; then
            continue
        fi
        if grep -Eq '^flags:.*(module|raw|async)|^[[:space:]]*-[[:space:]]*(module|raw|async)[[:space:]]*$' \
            <<<"$metadata"; then
            continue
        fi
        printf '%s\n' "$test_path"
    done < <(tagged_paths)
}

program_body() {
    local test_path=$1
    awk '/^---\*\/$/ { body=1; next } body { print }' "$suite/$test_path"
}

source_exclusion_records() {
    local test_path body category
    while IFS= read -r test_path; do
        body=$(program_body "$test_path")
        category=
        if LC_ALL=C grep -Eqi '(^|[^[:alnum:]_$])eval([^[:alnum:]_$]|$)' <<<"$body"; then
            category=eval
        elif LC_ALL=C grep -Eq '(^|[^[:alnum:]_$])(new[[:space:]]+)?Function[[:space:]]*\(' <<<"$body"; then
            category=dynamic-function
        elif LC_ALL=C grep -Eq '(^|[^.[:alnum:]_$])#[[:alnum:]_$\\]+[[:space:]]*\(' <<<"$body"; then
            category=private-method-or-accessor
        elif LC_ALL=C grep -Eq '(^|[^[:alnum:]_$])Proxy([^[:alnum:]_$]|$)' <<<"$body"; then
            category=proxy
        elif LC_ALL=C grep -Eq '(^|[^[:alnum:]_$])(await|yield)([^[:alnum:]_$]|$)' <<<"$body"; then
            category=await-yield-grammar
        fi
        if [[ -n "$category" ]]; then
            printf '%s\t%s\n' "$category" "$test_path"
        fi
    done
}

source_paths_for_category() {
    local category=$1
    awk -F'\t' -v category="$category" '$1 == category { print $2 }'
}

derive_features() {
    local test_path
    while IFS= read -r test_path; do
        metadata_block "$test_path" | awk '
            /^features:/ {
                sub(/^features:[[:space:]]*\[/, "")
                sub(/\][[:space:]]*$/, "")
                count=split($0, values, /,[[:space:]]*/)
                for (i=1; i<=count; i++) if (values[i] != "") print values[i]
            }
        '
    done < <(manifest_paths)
}

derive_negative_tests() {
    local test_path
    while IFS= read -r test_path; do
        if metadata_block "$test_path" | grep -Fq 'negative:'; then
            printf '%s\n' "$test_path"
        fi
    done < <(manifest_paths)
}

metadata_has_flag() {
    local metadata=$1
    local flag=$2
    grep -Eq "^flags:.*(^|[,[[:space:]])${flag}([],[[:space:]]|$)|^[[:space:]]*-[[:space:]]*${flag}[[:space:]]*$" \
        <<<"$metadata"
}

verify_quickjs_oracle() {
    local runner=$source_dir/run-test262
    local output test_path
    local -a paths=()
    if [[ ! -x "$runner" ]]; then
        "${MAKE:-make}" -C "$source_dir" run-test262 >&2
    fi
    while IFS= read -r test_path; do
        paths+=("test262/$test_path")
    done < <(manifest_paths)
    if ! output=$(
        cd -- "$source_dir"
        ./run-test262 -a -m -c test262.conf -f "${paths[@]}" 2>&1
    ); then
        printf '%s\n' "$output" >&2
        echo "error: pinned QuickJS could not execute the private-class-fields cohort" >&2
        exit 1
    fi
    if grep -Eq '(^|[[:space:]])FAILED($|[[:space:]])' <<<"$output" \
        || ! grep -Fq "Average memory statistics for $expected_quickjs_passes tests:" <<<"$output"; then
        printf '%s\n' "$output" >&2
        echo "error: pinned QuickJS no longer passes the complete private-class-fields cohort" >&2
        exit 1
    fi
}

cd -- "$root"
suite=$("$script_dir/prepare-test262.sh")
source_dir=$(dirname -- "$suite")

expected_quickjs=$(read_value quickjs)
expected_test262=$(read_value test262)
expected_patch=$(read_value test262_patch_sha256)
expected_config=$(read_value test262_config_sha256)
expected_metadata=$(read_value test262_metadata_sha256)
expected_global_profile=$(read_value global_oxide_profile_sha256)
expected_profile=$(read_value oxide_profile_sha256)
expected_schema=$(read_value schema)
expected_mode=$(read_value mode)
expected_timeout_ms=$(read_value timeout_ms)
expected_tagged_paths=$(read_value tagged_paths)
expected_metadata_excluded_paths=$(read_value metadata_excluded_paths)
expected_candidate_paths=$(read_value candidate_paths)
expected_source_excluded_paths=$(read_value source_excluded_paths)
expected_paths=$(read_value paths)
expected_variants=$(read_value variants)
expected_quickjs_passes=$(read_value quickjs_passes)
expected_runnable=$(read_value runnable)
expected_passes=$(read_value passes)
expected_failures=$(read_value failures)
expected_unsupported=$(read_value unsupported)
expected_skipped=$(read_value skipped)
expected_positive_paths=$(read_value positive_paths)
expected_negative_paths=$(read_value negative_paths)
expected_private_instance_paths=$(read_value private_instance_paths)
expected_private_static_paths=$(read_value private_static_paths)
expected_private_in_paths=$(read_value private_in_paths)
expected_generated_paths=$(read_value generated_paths)
expected_plain_paths=$(read_value plain_paths)
expected_property_helper_paths=$(read_value property_helper_paths)
expected_eval_excluded_paths=$(read_value eval_excluded_paths)
expected_dynamic_function_excluded_paths=$(read_value dynamic_function_excluded_paths)
expected_private_method_accessor_excluded_paths=$(read_value private_method_accessor_excluded_paths)
expected_proxy_excluded_paths=$(read_value proxy_excluded_paths)
expected_await_yield_excluded_paths=$(read_value await_yield_excluded_paths)
expected_tagged=$(read_value tagged_sha256)
expected_metadata_excluded=$(read_value metadata_excluded_sha256)
expected_candidate=$(read_value candidate_sha256)
expected_source_excluded=$(read_value source_excluded_sha256)
expected_eval_excluded=$(read_value eval_excluded_sha256)
expected_dynamic_function_excluded=$(read_value dynamic_function_excluded_sha256)
expected_private_method_accessor_excluded=$(read_value private_method_accessor_excluded_sha256)
expected_proxy_excluded=$(read_value proxy_excluded_sha256)
expected_await_yield_excluded=$(read_value await_yield_excluded_sha256)
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
    || "$expected_global_profile" != "1860224ce1e828406f4869b66b3f1964f96fad85e4eab6ba7fecb256b4b6c2f2" \
    || "$expected_profile" != "c03c22a7ea0d767536c77f1720b5c87766b06759d8a42a6e7b9ec3069633ffa2" \
    || "$expected_schema" != "test262-canonical-classified-v2" \
    || "$expected_mode" != "both" \
    || "$expected_timeout_ms" != "30000" \
    || "$expected_tagged_paths" != "1481" \
    || "$expected_metadata_excluded_paths" != "714" \
    || "$expected_candidate_paths" != "767" \
    || "$expected_source_excluded_paths" != "137" \
    || "$expected_paths" != "630" \
    || "$expected_variants" != "1260" \
    || "$expected_quickjs_passes" != "630" \
    || "$expected_runnable" != "1260" \
    || "$expected_passes" != "1260" \
    || "$expected_failures" != "0" \
    || "$expected_unsupported" != "0" \
    || "$expected_skipped" != "0" \
    || "$expected_positive_paths" != "405" \
    || "$expected_negative_paths" != "225" \
    || "$expected_private_instance_paths" != "465" \
    || "$expected_private_static_paths" != "167" \
    || "$expected_private_in_paths" != "11" \
    || "$expected_generated_paths" != "519" \
    || "$expected_plain_paths" != "111" \
    || "$expected_property_helper_paths" != "210" \
    || "$expected_eval_excluded_paths" != "96" \
    || "$expected_dynamic_function_excluded_paths" != "2" \
    || "$expected_private_method_accessor_excluded_paths" != "33" \
    || "$expected_proxy_excluded_paths" != "3" \
    || "$expected_await_yield_excluded_paths" != "3" ]]; then
    echo "error: private-class-fields baseline metadata drifted" >&2
    exit 1
fi

actual_paths=$(manifest_paths | wc -l | tr -d '[:space:]')
unique_paths=$(manifest_paths | LC_ALL=C sort -u | wc -l | tr -d '[:space:]')
if [[ "$actual_paths" != "$expected_paths" || "$unique_paths" != "$expected_paths" ]]; then
    echo "error: private-class-fields manifest cardinality drifted" >&2
    exit 1
fi
manifest_paths | LC_ALL=C sort -c
actual_manifest=$(manifest_paths | sha256_stream)
if [[ "$actual_manifest" != "$expected_manifest" \
    || "$(sha256_file "$manifest")" != "$expected_manifest_file" ]]; then
    echo "error: private-class-fields manifest content drifted" >&2
    exit 1
fi

tagged_inventory=$(tagged_paths)
actual_tagged_paths=$(printf '%s\n' "$tagged_inventory" | wc -l | tr -d '[:space:]')
actual_tagged=$(printf '%s\n' "$tagged_inventory" | sha256_stream)
candidate_inventory=$(derive_candidate_paths)
actual_candidate_paths=$(printf '%s\n' "$candidate_inventory" | wc -l | tr -d '[:space:]')
actual_candidate=$(printf '%s\n' "$candidate_inventory" | sha256_stream)
metadata_excluded_inventory=$(comm -23 \
    <(printf '%s\n' "$tagged_inventory") \
    <(printf '%s\n' "$candidate_inventory"))
actual_metadata_excluded_paths=$(printf '%s\n' "$metadata_excluded_inventory" | wc -l | tr -d '[:space:]')
actual_metadata_excluded=$(printf '%s\n' "$metadata_excluded_inventory" | sha256_stream)
source_records_inventory=$(printf '%s\n' "$candidate_inventory" | source_exclusion_records)
source_excluded_inventory=$(printf '%s\n' "$source_records_inventory" | cut -f2 | LC_ALL=C sort)
actual_source_excluded_paths=$(printf '%s\n' "$source_excluded_inventory" | wc -l | tr -d '[:space:]')
unique_source_excluded_paths=$(printf '%s\n' "$source_excluded_inventory" | LC_ALL=C sort -u | wc -l | tr -d '[:space:]')
actual_source_excluded=$(printf '%s\n' "$source_excluded_inventory" | sha256_stream)
if [[ "$actual_tagged_paths" != "$expected_tagged_paths" \
    || "$actual_tagged" != "$expected_tagged" \
    || "$actual_metadata_excluded_paths" != "$expected_metadata_excluded_paths" \
    || "$actual_metadata_excluded" != "$expected_metadata_excluded" \
    || "$actual_candidate_paths" != "$expected_candidate_paths" \
    || "$actual_candidate" != "$expected_candidate" \
    || "$actual_source_excluded_paths" != "$expected_source_excluded_paths" \
    || "$unique_source_excluded_paths" != "$expected_source_excluded_paths" \
    || "$actual_source_excluded" != "$expected_source_excluded" ]]; then
    echo "error: private-class-fields candidate or exclusion inventory drifted" >&2
    exit 1
fi

for category in eval dynamic-function private-method-or-accessor proxy await-yield-grammar; do
    category_inventory=$(printf '%s\n' "$source_records_inventory" | source_paths_for_category "$category")
    category_paths=$(printf '%s\n' "$category_inventory" | wc -l | tr -d '[:space:]')
    category_sha=$(printf '%s\n' "$category_inventory" | sha256_stream)
    case "$category" in
        eval)
            wanted_paths=$expected_eval_excluded_paths
            wanted_sha=$expected_eval_excluded
            ;;
        dynamic-function)
            wanted_paths=$expected_dynamic_function_excluded_paths
            wanted_sha=$expected_dynamic_function_excluded
            ;;
        private-method-or-accessor)
            wanted_paths=$expected_private_method_accessor_excluded_paths
            wanted_sha=$expected_private_method_accessor_excluded
            ;;
        proxy)
            wanted_paths=$expected_proxy_excluded_paths
            wanted_sha=$expected_proxy_excluded
            ;;
        await-yield-grammar)
            wanted_paths=$expected_await_yield_excluded_paths
            wanted_sha=$expected_await_yield_excluded
            ;;
    esac
    if [[ "$category_paths" != "$wanted_paths" || "$category_sha" != "$wanted_sha" ]]; then
        echo "error: private-class-fields $category exclusion inventory drifted" >&2
        exit 1
    fi
done

diff -u \
    <(comm -23 \
        <(printf '%s\n' "$candidate_inventory") \
        <(printf '%s\n' "$source_excluded_inventory")) \
    <(manifest_paths)

if [[ "$(sha256_file "$global_profile")" != "$expected_global_profile" \
    || "$(sha256_file "$admission_profile")" != "$expected_profile" ]]; then
    echo "error: private-class-fields capability profile drifted" >&2
    exit 1
fi
if awk '
    /^\[features\]$/ { inside=1; next }
    /^\[/ { inside=0 }
    inside && ($0 == "class-fields-private" ||
        $0 == "class-static-fields-private" ||
        $0 == "class-fields-private-in") {
        found=1
    }
    END { exit !found }
' "$global_profile"; then
    echo "error: global Test262 profile must remain fail-closed for private fields" >&2
    exit 1
fi

diff -u \
    <(profile_section features | LC_ALL=C sort) \
    <(derive_features | LC_ALL=C sort -u)
diff -u \
    <(profile_section audited-negative-tests | LC_ALL=C sort) \
    <(derive_negative_tests | LC_ALL=C sort)

private_instance_paths=0
private_static_paths=0
private_in_paths=0
generated_paths=0
plain_paths=0
property_helper_paths=0
positive_paths=0
negative_paths=0
while IFS= read -r test_path; do
    if [[ ! -f "$suite/$test_path" ]]; then
        echo "error: pinned private-class-fields path is missing: $test_path" >&2
        exit 1
    fi
    metadata=$(metadata_block "$test_path")
    features=$(metadata_features "$test_path")
    if ! grep -q '^features: \[' <<<"$metadata"; then
        echo "error: private-class-fields path gained non-inline features metadata: $test_path" >&2
        exit 1
    fi
    if ! grep -Eq '^(class-fields-private|class-static-fields-private|class-fields-private-in)$' \
        <<<"$features"; then
        echo "error: private-class-fields path lost every target feature tag: $test_path" >&2
        exit 1
    fi
    if grep -Eq '^(class-methods-private|class-static-methods-private|async-functions|async-iteration|generators|destructuring-binding|logical-assignment-operators|optional-chaining|exponentiation|BigInt|Proxy|cross-realm|nonextensible-applies-to-private|class-static-block)$' \
        <<<"$features"; then
        echo "error: private-class-fields path crossed an audited adjacent-feature frontier: $test_path" >&2
        exit 1
    fi
    if grep -Eq '^flags:.*(module|raw|async)|^[[:space:]]*-[[:space:]]*(module|raw|async)[[:space:]]*$' <<<"$metadata"; then
        echo "error: private-class-fields path flags left the audited scope: $test_path" >&2
        exit 1
    fi

    if grep -Fxq 'class-fields-private' <<<"$features"; then
        private_instance_paths=$((private_instance_paths + 1))
    fi
    if grep -Fxq 'class-static-fields-private' <<<"$features"; then
        private_static_paths=$((private_static_paths + 1))
    fi
    if grep -Fxq 'class-fields-private-in' <<<"$features"; then
        private_in_paths=$((private_in_paths + 1))
    fi

    flag_line=$(grep '^flags:' <<<"$metadata" || true)
    case "$flag_line" in
        "")
            plain_paths=$((plain_paths + 1))
            ;;
        "flags: [generated]")
            generated_paths=$((generated_paths + 1))
            ;;
        *)
            echo "error: private-class-fields flags drifted: $test_path: $flag_line" >&2
            exit 1
            ;;
    esac

    if grep -Fq 'negative:' <<<"$metadata"; then
        if ! grep -Fq '  phase: parse' <<<"$metadata" \
            || ! grep -Fq '  type: SyntaxError' <<<"$metadata"; then
            echo "error: private-class-fields negative provenance drifted: $test_path" >&2
            exit 1
        fi
        negative_paths=$((negative_paths + 1))
    else
        positive_paths=$((positive_paths + 1))
    fi
    if grep -q '^includes:' <<<"$metadata"; then
        if ! grep -Fxq 'includes: [propertyHelper.js]' <<<"$metadata"; then
            echo "error: private-class-fields harness dependency drifted: $test_path" >&2
            exit 1
        fi
        property_helper_paths=$((property_helper_paths + 1))
    fi

    body=$(program_body "$test_path")
    if LC_ALL=C grep -Eqi '(^|[^[:alnum:]_$])eval([^[:alnum:]_$]|$)' <<<"$body" \
        || LC_ALL=C grep -Eq '(^|[^[:alnum:]_$])(new[[:space:]]+)?Function[[:space:]]*\(' <<<"$body" \
        || LC_ALL=C grep -Eq '(^|[^.[:alnum:]_$])#[[:alnum:]_$\\]+[[:space:]]*\(' <<<"$body" \
        || LC_ALL=C grep -Eq '(^|[^[:alnum:]_$])Proxy([^[:alnum:]_$]|$)' <<<"$body" \
        || LC_ALL=C grep -Eq '(^|[^[:alnum:]_$])(await|yield)([^[:alnum:]_$]|$)' <<<"$body"; then
        echo "error: private-class-fields manifest crossed a source-audited frontier: $test_path" >&2
        exit 1
    fi
done < <(manifest_paths)

if [[ "$private_instance_paths" != "$expected_private_instance_paths" \
    || "$private_static_paths" != "$expected_private_static_paths" \
    || "$private_in_paths" != "$expected_private_in_paths" \
    || "$generated_paths" != "$expected_generated_paths" \
    || "$plain_paths" != "$expected_plain_paths" \
    || "$property_helper_paths" != "$expected_property_helper_paths" \
    || "$positive_paths" != "$expected_positive_paths" \
    || "$negative_paths" != "$expected_negative_paths" ]]; then
    echo "error: private-class-fields metadata composition drifted" >&2
    exit 1
fi

verify_quickjs_oracle
if "$check_only"; then
    printf 'private-class-fields inputs verified: QuickJS %s passes %s paths; Oxide admission gate expands to %s variants\n' \
        "$expected_quickjs" "$expected_quickjs_passes" "$expected_variants"
    exit 0
fi

rm -f -- "$report" "$json_report"
run_output=$(cargo run --locked --release --quiet --bin run-test262 -- \
    --suite "$suite" \
    --config "$source_dir/test262.conf" \
    --oxide-profile "$admission_profile" \
    --manifest "$manifest" \
    --report "$report" \
    --mode "$expected_mode" \
    --workers "$workers" \
    --timeout-ms "$expected_timeout_ms")
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
    echo "error: private-class-fields report metadata drifted" >&2
    exit 1
fi

diff -u \
    <(profile_section features | LC_ALL=C sort) \
    <(awk -F'\t' '
        !/^#/ && !($1 == "path" && $2 == "variant") {
            count=split($4, features, ",")
            for (i=1; i<=count; i++) if (features[i] != "") print features[i]
        }
    ' "$report" | LC_ALL=C sort -u)
diff -u \
    <(while IFS= read -r test_path; do
        metadata=$(metadata_block "$test_path")
        if metadata_has_flag "$metadata" noStrict; then
            printf '%s\tsloppy\n' "$test_path"
        elif metadata_has_flag "$metadata" onlyStrict; then
            printf '%s\tstrict\n' "$test_path"
        else
            printf '%s\tsloppy\n%s\tstrict\n' "$test_path" "$test_path"
        fi
    done < <(manifest_paths) | LC_ALL=C sort) \
    <(awk -F'\t' '!/^#/ && !($1 == "path" && $2 == "variant") { print $1 "\t" $2 }' "$report" | LC_ALL=C sort)

actual_keys=$(awk -F'\t' '!/^#/ && !($1 == "path" && $2 == "variant") { print $1 "\t" $2 }' "$report" | LC_ALL=C sort | sha256_stream)
actual_passes=$(awk -F'\t' '!/^#/ && !($1 == "path" && $2 == "variant") && $7 == "pass" { count++ } END { print count + 0 }' "$report")
actual_unsupported=$(awk -F'\t' '!/^#/ && !($1 == "path" && $2 == "variant") && $7 ~ /^unsupported-/ { count++ } END { print count + 0 }' "$report")
actual_skipped=$(awk -F'\t' '!/^#/ && !($1 == "path" && $2 == "variant") && $7 ~ /^skipped-/ { count++ } END { print count + 0 }' "$report")
actual_failures=$((actual_variants - actual_passes - actual_unsupported - actual_skipped))
actual_nonpass=$(awk -F'\t' '!/^#/ && !($1 == "path" && $2 == "variant") && $7 != "pass" { print $1 "\t" $2 "\t" $7 "\t" $8 "\t" $9 "\t" $10 }' "$report" | sha256_stream)
actual_summary=$(tail -n 1 "$report" | sed 's/^# summary //')
runner_summary=$(printf '%s\n' "$run_output" | awk '/^Test262: total=/ { print; found=1 } END { if (!found) exit 1 }')
expected_runner_summary="Test262: total=$expected_variants pass=$expected_passes fail=$expected_failures unsupported=$expected_unsupported skipped=$expected_skipped"
if [[ "$runner_summary" != "$expected_runner_summary" ]]; then
    echo "error: private-class-fields runner summary drifted: $runner_summary" >&2
    exit 1
fi

if [[ "$actual_passes" != "$expected_passes" \
    || "$actual_failures" != "$expected_failures" \
    || "$actual_unsupported" != "$expected_unsupported" \
    || "$actual_skipped" != "$expected_skipped" \
    || "$actual_keys" != "$expected_keys" \
    || "$actual_nonpass" != "$expected_nonpass" \
    || "$actual_summary" != "$expected_summary" \
    || "$(sha256_file "$report")" != "$expected_tsv" \
    || "$(sha256_file "$json_report")" != "$expected_jsonl" ]]; then
    echo "error: private-class-fields Test262 classified vector drifted" >&2
    awk -F'\t' '
        !/^#/ && !($1 == "path" && $2 == "variant") && $7 != "pass" {
            print $1 "\t" $2 "\t" $7 "\t" $8 "\t" $9 "\t" $10
        }
    ' "$report" >&2
    exit 1
fi

printf 'private-class-fields Test262 gate is exact: %s/%s pass across %s audited paths\n' \
    "$actual_passes" "$actual_variants" "$expected_paths"
