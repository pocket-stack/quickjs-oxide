#!/usr/bin/env bash
# Reproduce the dependency-audited R3j synchronous private-accessor cohort.

set -euo pipefail
export TZ=America/Los_Angeles

script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
root=$(CDPATH= cd -- "$script_dir/.." && pwd)
baseline=tests/test262-class-private-accessors-baseline.txt
manifest=tests/test262-class-private-accessors.txt
admission_profile=tests/test262-class-private-accessors.conf
global_profile=compat/test262-oxide.conf
report=target/test262-class-private-accessors.tsv
json_report=target/test262-class-private-accessors.jsonl
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
        'class-methods-private|class-static-methods-private' \
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
            exit
        }
        /^features:[[:space:]]*$/ { inside=1; next }
        inside && /^[[:space:]]*-[[:space:]]*/ {
            sub(/^[[:space:]]*-[[:space:]]*/, "")
            print
            next
        }
        inside { exit }
    '
}

features_leave_minimum_scope() {
    grep -Ev '^(class|class-fields-private|class-fields-private-in|class-fields-public|class-methods-private|class-static-fields-private|class-static-fields-public|class-static-methods-private)$' \
        | grep -q .
}

derive_candidate_paths() {
    local test_path metadata features
    while IFS= read -r test_path; do
        metadata=$(metadata_block "$test_path")
        features=$(metadata_features "$test_path")
        if ! grep -Eq '^(class-methods-private|class-static-methods-private)$' \
            <<<"$features"; then
            continue
        fi
        if features_leave_minimum_scope <<<"$features"; then
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
        elif LC_ALL=C grep -Eq '(^|[^[:alnum:]_$])Proxy([^[:alnum:]_$]|$)' <<<"$body"; then
            category=proxy
        elif LC_ALL=C grep -Eq '(^|[^[:alnum:]_$])(await|yield)([^[:alnum:]_$]|$)' <<<"$body"; then
            category=await-yield
        elif LC_ALL=C grep -Eq '(^|[^[:alnum:]_$])async([^[:alnum:]_$]|$)' <<<"$body"; then
            category=async-syntax
        elif LC_ALL=C grep -Eq '(function[[:space:]]*\*|\*[[:space:]]*#[[:alnum:]_$\\]+)' <<<"$body"; then
            category=generator-syntax
        elif LC_ALL=C grep -Eq '(^|[;}])[[:space:]]*static[[:space:]]*\{' <<<"$body"; then
            category=static-block
        elif LC_ALL=C grep -Fq '?.' <<<"$body"; then
            category=optional-chaining
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

derive_accessor_paths() {
    local test_path body
    while IFS= read -r test_path; do
        body=$(program_body "$test_path")
        if LC_ALL=C grep -Eqi '(accessor|getter|setter)' <<<"$test_path" \
            || LC_ALL=C grep -Eq '(^|[^[:alnum:]_$])(get|set)[[:space:]]+#[^[:space:](]+[[:space:]]*\(' <<<"$body"; then
            printf '%s\n' "$test_path"
        fi
    done
}

derive_features() {
    local test_path
    while IFS= read -r test_path; do
        metadata_features "$test_path"
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

derive_positive_tests() {
    local test_path
    while IFS= read -r test_path; do
        if ! metadata_block "$test_path" | grep -Fq 'negative:'; then
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
    local -a files=()
    if [[ ! -x "$runner" ]]; then
        "${MAKE:-make}" -C "$source_dir" run-test262 >&2
    fi
    while IFS= read -r test_path; do
        files+=("test262/$test_path")
    done < <(manifest_paths)
    if ! output=$(
        cd -- "$source_dir"
        ./run-test262 -a -m -c test262.conf -f "${files[@]}" 2>&1
    ); then
        printf '%s\n' "$output" >&2
        echo "error: pinned QuickJS could not execute the private-accessor cohort" >&2
        exit 1
    fi
    if grep -Eq '(^|[[:space:]])FAILED($|[[:space:]])' <<<"$output" \
        || ! grep -Fq "Average memory statistics for $expected_quickjs_passes tests:" <<<"$output"; then
        printf '%s\n' "$output" >&2
        echo "error: pinned QuickJS no longer passes the complete private-accessor cohort" >&2
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
expected_paths=$(read_value paths)
expected_variants=$(read_value variants)
expected_quickjs_passes=$(read_value quickjs_passes)

if [[ "$expected_quickjs" != "2026-06-04" \
    || "$expected_test262" != "5c8206929d81b2d3d727ca6aac56c18358c8d790" \
    || "$expected_patch" != "f4b23b04641d438df0826fb17d7a5db276af2bdb085b42cc09aa8d50e0da9ba3" \
    || "$expected_config" != "79c64748ff1182baf5433d0a8378e3666738a785d02faf71f0d459ed42ae897b" \
    || "$expected_metadata" != "a37219960819e56a5c5c1723d31d6a33095c778bf5347385187fde96f927a06a" \
    || "$expected_schema" != "test262-canonical-classified-v2" \
    || "$expected_mode" != "both" \
    || "$expected_timeout_ms" != "30000" \
    || "$(read_value tagged_paths)" != "3208" \
    || "$(read_value tagged_variants)" != "6382" \
    || "$(read_value metadata_excluded_paths)" != "2557" \
    || "$(read_value metadata_excluded_variants)" != "5092" \
    || "$(read_value candidate_paths)" != "651" \
    || "$(read_value candidate_variants)" != "1290" \
    || "$(read_value source_excluded_paths)" != "79" \
    || "$(read_value source_excluded_variants)" != "146" \
    || "$(read_value minimum_sync_paths)" != "572" \
    || "$(read_value minimum_sync_variants)" != "1144" \
    || "$(read_value pre_accessor_paths)" != "323" \
    || "$(read_value pre_accessor_variants)" != "638" \
    || "$(read_value accessor_source_excluded_paths)" != "18" \
    || "$(read_value accessor_source_excluded_variants)" != "28" \
    || "$expected_paths" != "305" \
    || "$expected_variants" != "610" \
    || "$expected_quickjs_passes" != "305" \
    || "$(read_value positive_paths)" != "229" \
    || "$(read_value negative_paths)" != "76" \
    || "$(read_value manifest_sha256)" != "ca77913172666cbe4e74a6476f7f4d87383e801260b2c5b80932dc15e8e98cd6" ]]; then
    echo "error: private-accessor baseline identity drifted" >&2
    exit 1
fi

actual_paths=$(manifest_paths | wc -l | tr -d '[:space:]')
unique_paths=$(manifest_paths | LC_ALL=C sort -u | wc -l | tr -d '[:space:]')
if [[ "$actual_paths" != "$expected_paths" || "$unique_paths" != "$expected_paths" ]]; then
    echo "error: private-accessor manifest cardinality drifted" >&2
    exit 1
fi
manifest_paths | LC_ALL=C sort -c
actual_manifest=$(manifest_paths | sha256_stream)
if [[ "$actual_manifest" != "$(read_value manifest_sha256)" \
    || "$(sha256_file "$manifest")" != "$(read_value manifest_file_sha256)" ]]; then
    echo "error: private-accessor manifest content drifted" >&2
    exit 1
fi

tagged_inventory=$(tagged_paths)
candidate_inventory=$(derive_candidate_paths)
metadata_excluded_inventory=$(comm -23 \
    <(printf '%s\n' "$tagged_inventory") \
    <(printf '%s\n' "$candidate_inventory"))
source_records_inventory=$(printf '%s\n' "$candidate_inventory" | source_exclusion_records)
source_excluded_inventory=$(printf '%s\n' "$source_records_inventory" | cut -f2 | LC_ALL=C sort)
minimum_sync_inventory=$(comm -23 \
    <(printf '%s\n' "$candidate_inventory") \
    <(printf '%s\n' "$source_excluded_inventory"))
pre_accessor_inventory=$(printf '%s\n' "$candidate_inventory" | derive_accessor_paths)
accessor_source_excluded_inventory=$(comm -12 \
    <(printf '%s\n' "$pre_accessor_inventory") \
    <(printf '%s\n' "$source_excluded_inventory"))
accessor_inventory=$(printf '%s\n' "$minimum_sync_inventory" | derive_accessor_paths)
method_inventory=$(comm -23 \
    <(printf '%s\n' "$minimum_sync_inventory") \
    <(printf '%s\n' "$accessor_inventory"))

inventory_count() {
    inventory_stream "$1" | wc -l | tr -d '[:space:]'
}

inventory_stream() {
    if [[ -n "${1-}" ]]; then
        printf '%s\n' "$1"
    fi
}

inventory_sha256() {
    inventory_stream "$1" | sha256_stream
}

if [[ "$(inventory_count "$tagged_inventory")" != "$(read_value tagged_paths)" \
    || "$(inventory_sha256 "$tagged_inventory")" != "$(read_value tagged_sha256)" \
    || "$(inventory_count "$metadata_excluded_inventory")" != "$(read_value metadata_excluded_paths)" \
    || "$(inventory_sha256 "$metadata_excluded_inventory")" != "$(read_value metadata_excluded_sha256)" \
    || "$(inventory_count "$candidate_inventory")" != "$(read_value candidate_paths)" \
    || "$(inventory_sha256 "$candidate_inventory")" != "$(read_value candidate_sha256)" \
    || "$(inventory_count "$source_excluded_inventory")" != "$(read_value source_excluded_paths)" \
    || "$(inventory_sha256 "$source_excluded_inventory")" != "$(read_value source_excluded_sha256)" \
    || "$(inventory_count "$minimum_sync_inventory")" != "$(read_value minimum_sync_paths)" \
    || "$(inventory_sha256 "$minimum_sync_inventory")" != "$(read_value minimum_sync_sha256)" \
    || "$(inventory_count "$pre_accessor_inventory")" != "$(read_value pre_accessor_paths)" \
    || "$(inventory_sha256 "$pre_accessor_inventory")" != "$(read_value pre_accessor_sha256)" \
    || "$(inventory_count "$accessor_source_excluded_inventory")" != "$(read_value accessor_source_excluded_paths)" \
    || "$(inventory_sha256 "$accessor_source_excluded_inventory")" != "$(read_value accessor_source_excluded_sha256)" \
    || "$(inventory_count "$accessor_inventory")" != "$expected_paths" \
    || "$(inventory_sha256 "$accessor_inventory")" != "$(read_value manifest_sha256)" \
    || "$(inventory_count "$method_inventory")" != "267" \
    || "$(inventory_sha256 "$method_inventory")" != "7ea0bbef5d3b5b27aa5e661574fbb0f53cc65fa785874bd1baabb1d83339b375" ]]; then
    echo "error: private-accessor candidate, exclusion, or partition inventory drifted" >&2
    exit 1
fi

diff -u <(printf '%s\n' "$accessor_inventory") <(manifest_paths)

for category in eval dynamic-function proxy await-yield async-syntax generator-syntax static-block optional-chaining; do
    category_inventory=$(printf '%s\n' "$source_records_inventory" | source_paths_for_category "$category")
    accessor_category_inventory=$(comm -12 \
        <(printf '%s\n' "$pre_accessor_inventory") \
        <(printf '%s\n' "$category_inventory"))
    category_key=${category//-/_}
    if [[ "$(inventory_count "$category_inventory")" != "$(read_value "${category_key}_excluded_paths")" \
        || "$(inventory_sha256 "$category_inventory")" != "$(read_value "${category_key}_excluded_sha256")" \
        || "$(inventory_count "$accessor_category_inventory")" != "$(read_value "accessor_${category_key}_excluded_paths")" \
        || "$(inventory_sha256 "$accessor_category_inventory")" != "$(read_value "accessor_${category_key}_excluded_sha256")" ]]; then
        echo "error: private-accessor $category exclusion inventory drifted" >&2
        exit 1
    fi
done

if [[ "$(sha256_file "$global_profile")" != "$expected_global_profile" \
    || "$(sha256_file "$admission_profile")" != "$expected_profile" ]]; then
    echo "error: private-accessor capability profile drifted" >&2
    exit 1
fi
if awk '
    /^\[features\]$/ { inside=1; next }
    /^\[/ { inside=0 }
    inside && ($0 == "class-methods-private" ||
        $0 == "class-static-methods-private") { found=1 }
    END { exit !found }
' "$global_profile"; then
    echo "error: global Test262 profile must remain fail-closed for private accessors" >&2
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
private_both_paths=0
private_in_paths=0
generated_paths=0
plain_paths=0
property_helper_paths=0
native_function_matcher_paths=0
brand_named_paths=0
positive_paths=0
negative_paths=0
while IFS= read -r test_path; do
    if [[ ! -f "$suite/$test_path" ]]; then
        echo "error: pinned private-accessor path is missing: $test_path" >&2
        exit 1
    fi
    metadata=$(metadata_block "$test_path")
    features=$(metadata_features "$test_path")
    if ! grep -q '^features: \[' <<<"$metadata" \
        || ! grep -Eq '^(class-methods-private|class-static-methods-private)$' <<<"$features" \
        || features_leave_minimum_scope <<<"$features"; then
        echo "error: private-accessor feature scope drifted: $test_path" >&2
        exit 1
    fi

    has_instance=false
    has_static=false
    if grep -Fxq 'class-methods-private' <<<"$features"; then
        private_instance_paths=$((private_instance_paths + 1))
        has_instance=true
    fi
    if grep -Fxq 'class-static-methods-private' <<<"$features"; then
        private_static_paths=$((private_static_paths + 1))
        has_static=true
    fi
    if "$has_instance" && "$has_static"; then
        private_both_paths=$((private_both_paths + 1))
    fi
    if grep -Fxq 'class-fields-private-in' <<<"$features"; then
        private_in_paths=$((private_in_paths + 1))
    fi

    flag_line=$(grep '^flags:' <<<"$metadata" || true)
    case "$flag_line" in
        "") plain_paths=$((plain_paths + 1)) ;;
        "flags: [generated]") generated_paths=$((generated_paths + 1)) ;;
        *)
            echo "error: private-accessor flags drifted: $test_path: $flag_line" >&2
            exit 1
            ;;
    esac

    if grep -Fq 'negative:' <<<"$metadata"; then
        if ! grep -Fq '  phase: parse' <<<"$metadata" \
            || ! grep -Fq '  type: SyntaxError' <<<"$metadata"; then
            echo "error: private-accessor negative provenance drifted: $test_path" >&2
            exit 1
        fi
        negative_paths=$((negative_paths + 1))
    else
        positive_paths=$((positive_paths + 1))
    fi

    include_line=$(grep '^includes:' <<<"$metadata" || true)
    case "$include_line" in
        "") ;;
        "includes: [propertyHelper.js]")
            property_helper_paths=$((property_helper_paths + 1))
            ;;
        "includes: [nativeFunctionMatcher.js]")
            native_function_matcher_paths=$((native_function_matcher_paths + 1))
            ;;
        *)
            echo "error: private-accessor harness dependency drifted: $test_path" >&2
            exit 1
            ;;
    esac
    if grep -qi 'brand' <<<"$test_path"; then
        brand_named_paths=$((brand_named_paths + 1))
    fi

    if [[ -n "$(printf '%s\n' "$test_path" | source_exclusion_records)" \
        || -z "$(printf '%s\n' "$test_path" | derive_accessor_paths)" ]]; then
        echo "error: private-accessor manifest crossed its source or accessor partition: $test_path" >&2
        exit 1
    fi
done < <(manifest_paths)

for pair in \
    "private_instance_paths:$private_instance_paths" \
    "private_static_paths:$private_static_paths" \
    "private_both_paths:$private_both_paths" \
    "private_in_paths:$private_in_paths" \
    "generated_paths:$generated_paths" \
    "plain_paths:$plain_paths" \
    "property_helper_paths:$property_helper_paths" \
    "native_function_matcher_paths:$native_function_matcher_paths" \
    "brand_named_paths:$brand_named_paths" \
    "positive_paths:$positive_paths" \
    "negative_paths:$negative_paths"; do
    key=${pair%%:*}
    actual=${pair#*:}
    if [[ "$actual" != "$(read_value "$key")" ]]; then
        echo "error: private-accessor metadata composition drifted for $key" >&2
        exit 1
    fi
done

if [[ "$(derive_positive_tests | sha256_stream)" != "$(read_value positive_sha256)" \
    || "$(derive_negative_tests | sha256_stream)" != "$(read_value negative_sha256)" ]]; then
    echo "error: private-accessor positive or negative inventory drifted" >&2
    exit 1
fi

verify_quickjs_oracle
if "$check_only"; then
    printf 'private-accessor inputs verified: QuickJS %s passes %s paths; Oxide admission gate expands to %s variants\n' \
        "$expected_quickjs" "$expected_quickjs_passes" "$expected_variants"
    exit 0
fi

pending_keys=()
for key in passes failures unsupported skipped keys_sha256 nonpass_sha256 tsv_sha256 jsonl_sha256 summary; do
    if [[ "$(read_value "$key")" == "PENDING" ]]; then
        pending_keys+=("$key")
    fi
done
if [[ ${#pending_keys[@]} -ne 0 ]]; then
    printf 'error: private-accessor Oxide baseline needs refresh before execution: %s\n' \
        "${pending_keys[*]}" >&2
    echo "run --check until the implementation is ready, then record the classified vector" >&2
    exit 1
fi

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
    echo "error: private-accessor report metadata drifted" >&2
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
    echo "error: private-accessor runner summary drifted: $runner_summary" >&2
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
    echo "error: private-accessor Test262 classified vector drifted" >&2
    awk -F'\t' '
        !/^#/ && !($1 == "path" && $2 == "variant") && $7 != "pass" {
            print $1 "\t" $2 "\t" $7 "\t" $8 "\t" $9 "\t" $10
        }
    ' "$report" >&2
    exit 1
fi

printf 'private-accessor Test262 gate is exact: %s/%s pass across %s audited paths\n' \
    "$actual_passes" "$actual_variants" "$expected_paths"
