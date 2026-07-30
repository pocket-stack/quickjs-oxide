#!/usr/bin/env bash
# Reproduce the dependency-audited optional-chaining Test262 focused gate.

set -euo pipefail
export TZ=America/Los_Angeles

script_dir=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd)
root=$(CDPATH='' cd -- "$script_dir/.." && pwd)
baseline=tests/test262-optional-chaining-baseline.txt
manifest=tests/test262-optional-chaining.txt
reason_only_manifest=tests/test262-optional-chaining-reason-only.txt
iterator_adjacency_manifest=tests/test262-optional-chaining-iterator-adjacency.txt
profile=tests/test262-optional-chaining.conf
report=target/test262-optional-chaining.tsv
json_report=target/test262-optional-chaining.jsonl
oracle_log=target/test262-optional-chaining-quickjs.log
workers=${TEST262_WORKERS:-8}

usage() {
    printf 'usage: %s [--check|--bless]\n' "${0##*/}"
    printf '  --check  verify frozen inventories/profile and pinned QuickJS only\n'
    printf '  --bless  record an Oxide baseline only when all focused variants pass\n'
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
    awk 'NF && $1 !~ /^#/ { print }' "$1"
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

negative_field() {
    local test_path=$1 key=$2
    metadata_block "$test_path" | awk -v key="$key" '
        /^negative:[[:space:]]*$/ { inside=1; next }
        inside && /^[^[:space:]]/ { exit }
        inside {
            line=$0
            sub(/^[[:space:]]+/, "", line)
            if (line ~ ("^" key ":[[:space:]]*")) {
                sub("^[^:]+:[[:space:]]*", "", line)
                print line
                exit
            }
        }
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
        echo "error: optional-chaining $name inventory drifted" >&2
        exit 1
    fi
}

variant_keys() {
    local test_path
    while IFS= read -r test_path; do
        [[ -z "$test_path" ]] && continue
        printf '%s\tsloppy\n%s\tstrict\n' "$test_path" "$test_path"
    done | LC_ALL=C sort
}

verify_key_inventory() {
    local name=$1 paths=$2 expected_count expected_hash keys actual_count actual_hash
    expected_count=$(read_value "${name}_variants")
    expected_hash=$(read_value "${name}_keys_sha256")
    keys=$(printf '%s\n' "$paths" | variant_keys)
    actual_count=$(printf '%s\n' "$keys" | sed '/^$/d' | wc -l | tr -d '[:space:]')
    actual_hash=$(printf '%s\n' "$keys" | sed '/^$/d' | sha256_stream)
    if [[ "$actual_count" != "$expected_count" || "$actual_hash" != "$expected_hash" ]]; then
        echo "error: optional-chaining $name variant-key inventory drifted" >&2
        exit 1
    fi
}

verify_quickjs_oracle() {
    local test_path actual_oracle_variants
    local -a oracle_files=()
    while IFS= read -r test_path; do
        [[ -z "$test_path" ]] && continue
        oracle_files+=("test262/$test_path")
    done < <(manifest_paths "$manifest")

    if ! (
        cd -- "$source_dir"
        ./run-test262 -m -c test262.conf -a -T "$workers" -f "${oracle_files[@]}"
    ) >"$root/$oracle_log" 2>&1; then
        echo "error: pinned QuickJS failed the optional-chaining manifest" >&2
        tail -n 80 "$root/$oracle_log" >&2
        exit 1
    fi
    actual_oracle_variants=$(awk '
        /^Average memory statistics for [0-9]+ tests:$/ {
            print $5
            found=1
        }
        END { if (!found) exit 1 }
    ' "$root/$oracle_log")
    if [[ "$actual_oracle_variants" != "$(read_value quickjs_variants)" ]] \
        || grep -Eq '(^|[[:space:]])FAILED($|[[:space:]])' "$root/$oracle_log"; then
        echo "error: pinned QuickJS optional-chaining oracle vector drifted" >&2
        tail -n 80 "$root/$oracle_log" >&2
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
    || "$(read_value r3bh_global_oxide_profile_sha256)" != "2bfad693206dd09934a4c95ca241c49c4997ad795b8f0016571aada9c2cf1804" \
    || "$(read_value oxide_profile_sha256)" != "42bdcf4005aafed999604c10db1298651875210ea2ee2d96569a3ec54a99e064" \
    || "$(read_value schema)" != "test262-canonical-classified-v2" \
    || "$(read_value mode)" != "both" \
    || "$(read_value timeout_ms)" != "30000" \
    || "$(read_value tagged_paths)" != "56" \
    || "$(read_value tagged_variants)" != "112" \
    || "$(read_value reason_only_paths)" != "4" \
    || "$(read_value reason_only_variants)" != "8" \
    || "$(read_value negative_paths)" != "26" \
    || "$(read_value negative_variants)" != "52" \
    || "$(read_value iterator_adjacency_paths)" != "14" \
    || "$(read_value iterator_adjacency_variants)" != "28" \
    || "$(read_value paths)" != "52" \
    || "$(read_value variants)" != "104" \
    || "$(read_value quickjs_variants)" != "104" \
    || "$(read_value features)" != "2" \
    || "$(read_value includes)" != "1" ]]; then
    echo "error: optional-chaining baseline identity drifted" >&2
    exit 1
fi

for ledger in "$manifest" "$reason_only_manifest" "$iterator_adjacency_manifest"; do
    manifest_paths "$ledger" | LC_ALL=C sort -c
    while IFS= read -r test_path; do
        [[ -f "$suite/$test_path" ]] \
            || { echo "error: pinned Test262 path is missing: $test_path" >&2; exit 1; }
    done < <(manifest_paths "$ledger")
done

profile_features=$(profile_section features)
profile_negatives=$(profile_section audited-negative-tests)
profile_section features | LC_ALL=C sort -c
profile_section audited-negative-tests | LC_ALL=C sort -c
[[ "$(profile_section execution)" == "async=true" ]] \
    || { echo "error: optional-chaining profile lost async execution" >&2; exit 1; }
[[ "$(sha256_file "$profile")" == "$(read_value oxide_profile_sha256)" ]] \
    || { echo "error: optional-chaining capability profile drifted" >&2; exit 1; }
[[ "$(printf '%s\n' "$profile_features" | wc -l | tr -d '[:space:]')" == "$(read_value profile_features)" \
    && "$(printf '%s\n' "$profile_features" | sha256_stream)" == "$(read_value profile_features_sha256)" \
    && "$(printf '%s\n' "$profile_negatives" | wc -l | tr -d '[:space:]')" == "$(read_value profile_negative_paths)" \
    && "$(printf '%s\n' "$profile_negatives" | sha256_stream)" == "$(read_value profile_negative_sha256)" ]] \
    || { echo "error: optional-chaining profile section inventory drifted" >&2; exit 1; }

tagged_inventory=$(
    git -C "$suite" grep -l -F 'optional-chaining' -- 'test/**/*.js' \
        | while IFS= read -r test_path; do
            if metadata_list "$test_path" features | grep -Fxq 'optional-chaining'; then
                printf '%s\n' "$test_path"
            fi
        done \
        | LC_ALL=C sort
)
verify_inventory tagged "$tagged_inventory"
verify_key_inventory tagged "$tagged_inventory"

base_features=$(comm -23 \
    <(printf '%s\n' "$profile_features") \
    <(printf '%s\n' optional-chaining))
[[ "$(printf '%s\n' "$base_features" | wc -l | tr -d '[:space:]')" == "$(read_value base_features)" \
    && "$(printf '%s\n' "$base_features" | sha256_stream)" == "$(read_value base_features_sha256)" ]] \
    || { echo "error: frozen R3bh base feature inventory drifted" >&2; exit 1; }
diff -u \
    <(printf '%s\n' optional-chaining) \
    <(comm -13 <(printf '%s\n' "$base_features") <(printf '%s\n' "$profile_features"))

reason_only_inventory=$(
    while IFS= read -r test_path; do
        missing=$(
            comm -23 \
                <(metadata_list "$test_path" features \
                    | grep -Fvx 'optional-chaining' \
                    | LC_ALL=C sort -u) \
                <(printf '%s\n' "$base_features")
        )
        if [[ -n "$missing" ]]; then
            printf '%s\n' "$test_path"
        fi
    done <<<"$tagged_inventory"
)
verify_inventory reason_only "$reason_only_inventory"
verify_key_inventory reason_only "$reason_only_inventory"
diff -u \
    <(printf '%s\n' "$reason_only_inventory") \
    <(manifest_paths "$reason_only_manifest")

derived_manifest=$(
    comm -23 \
        <(printf '%s\n' "$tagged_inventory") \
        <(printf '%s\n' "$reason_only_inventory")
)
diff -u \
    <(printf '%s\n' "$derived_manifest") \
    <(manifest_paths "$manifest")
actual_manifest_paths=$(printf '%s\n' "$derived_manifest" | sed '/^$/d' | wc -l | tr -d '[:space:]')
actual_manifest_hash=$(printf '%s\n' "$derived_manifest" | sed '/^$/d' | sha256_stream)
[[ "$actual_manifest_paths" == "$(read_value paths)" \
    && "$actual_manifest_hash" == "$(read_value manifest_sha256)" \
    && "$(sha256_file "$manifest")" == "$(read_value manifest_file_sha256)" ]] \
    || { echo "error: optional-chaining manifest file drifted" >&2; exit 1; }

negative_inventory=
feature_inventory=
include_inventory=
while IFS= read -r test_path; do
    metadata=$(metadata_block "$test_path")
    flag_line=$(grep '^flags:' <<<"$metadata" || true)
    if grep -Eq '(noStrict|onlyStrict|module|raw)' <<<"$flag_line"; then
        echo "error: optional-chaining path lost its two ordinary variants: $test_path" >&2
        exit 1
    fi
    features=$(metadata_list "$test_path" features)
    grep -Fxq 'optional-chaining' <<<"$features" \
        || { echo "error: selected path lost optional-chaining metadata: $test_path" >&2; exit 1; }
    feature_inventory+=$'\n'"$features"
    include_inventory+=$'\n'"$(metadata_list "$test_path" includes)"
    phase=$(negative_field "$test_path" phase)
    if [[ -n "$phase" ]]; then
        [[ "$phase" == "parse" && "$(negative_field "$test_path" type)" == "SyntaxError" ]] \
            || { echo "error: optional-chaining negative provenance drifted: $test_path" >&2; exit 1; }
        negative_inventory+=$'\n'"$test_path"
    fi
done < <(manifest_paths "$manifest")

negative_inventory=$(printf '%s\n' "$negative_inventory" | sed '/^$/d' | LC_ALL=C sort)
feature_inventory=$(printf '%s\n' "$feature_inventory" | sed '/^$/d' | LC_ALL=C sort -u)
include_inventory=$(printf '%s\n' "$include_inventory" | sed '/^$/d' | LC_ALL=C sort -u)
verify_inventory negative "$negative_inventory"
verify_key_inventory negative "$negative_inventory"
[[ "$(printf '%s\n' "$feature_inventory" | wc -l | tr -d '[:space:]')" == "$(read_value features)" \
    && "$(printf '%s\n' "$feature_inventory" | sha256_stream)" == "$(read_value features_sha256)" \
    && "$(printf '%s\n' "$include_inventory" | wc -l | tr -d '[:space:]')" == "$(read_value includes)" \
    && "$(printf '%s\n' "$include_inventory" | sha256_stream)" == "$(read_value includes_sha256)" ]] \
    || { echo "error: optional-chaining focused metadata inventory drifted" >&2; exit 1; }
[[ -z "$(comm -23 <(printf '%s\n' "$feature_inventory") <(printf '%s\n' "$profile_features"))" ]] \
    || { echo "error: optional-chaining metadata exceeds the scoped profile" >&2; exit 1; }
[[ -z "$(comm -23 <(printf '%s\n' "$negative_inventory") <(printf '%s\n' "$profile_negatives"))" ]] \
    || { echo "error: optional-chaining negative path is absent from the scoped profile" >&2; exit 1; }

base_negatives=$(
    comm -23 \
        <(printf '%s\n' "$profile_negatives") \
        <(printf '%s\n' "$negative_inventory")
)
[[ "$(printf '%s\n' "$base_negatives" | wc -l | tr -d '[:space:]')" == "$(read_value base_negative_paths)" \
    && "$(printf '%s\n' "$base_negatives" | sha256_stream)" == "$(read_value base_negative_sha256)" ]] \
    || { echo "error: frozen R3bh base negative provenance drifted" >&2; exit 1; }
diff -u \
    <(printf '%s\n' "$negative_inventory") \
    <(comm -13 <(printf '%s\n' "$base_negatives") <(printf '%s\n' "$profile_negatives"))

iterator_adjacency_inventory=$(
    git -C "$suite" grep -l -F 'iterator-helpers' -- 'test/**/*.js' \
        | while IFS= read -r test_path; do
            features=$(metadata_list "$test_path" features)
            if grep -Fxq 'iterator-helpers' <<<"$features" \
                && ! grep -Fxq 'optional-chaining' <<<"$features" \
                && grep -Fq '?.' < <(program_body "$test_path"); then
                printf '%s\n' "$test_path"
            fi
        done \
        | LC_ALL=C sort
)
verify_inventory iterator_adjacency "$iterator_adjacency_inventory"
verify_key_inventory iterator_adjacency "$iterator_adjacency_inventory"
diff -u \
    <(printf '%s\n' "$iterator_adjacency_inventory") \
    <(manifest_paths "$iterator_adjacency_manifest")

focused_keys=$(manifest_paths "$manifest" | variant_keys)
[[ "$(printf '%s\n' "$focused_keys" | wc -l | tr -d '[:space:]')" == "$(read_value variants)" \
    && "$(printf '%s\n' "$focused_keys" | sha256_stream)" == "$(read_value keys_sha256)" ]] \
    || { echo "error: optional-chaining focused variant-key inventory drifted" >&2; exit 1; }

verify_quickjs_oracle
if "$check_only"; then
    printf 'Optional chaining inputs verified: %s tagged - %s reason-only = %s focused paths; QuickJS %s/%s; Iterator adjacency %s paths\n' \
        "$(read_value tagged_paths)" \
        "$(read_value reason_only_paths)" \
        "$(read_value paths)" \
        "$(read_value quickjs_variants)" \
        "$(read_value variants)" \
        "$(read_value iterator_adjacency_paths)"
    exit 0
fi

pending_keys=()
for key in runnable passes failures unsupported skipped nonpass_sha256 tsv_sha256 jsonl_sha256 summary; do
    if [[ "$(read_value "$key")" == "PENDING" ]]; then
        pending_keys+=("$key")
    fi
done
if [[ ${#pending_keys[@]} -ne 0 ]] && ! "$bless"; then
    printf 'error: optional-chaining Oxide baseline needs refresh after implementation: %s\n' \
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
    echo "error: optional-chaining report metadata drifted" >&2
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
    <(manifest_paths "$manifest") \
    <(awk -F'\t' '!/^#/ && !($1 == "path" && $2 == "variant") { print $1 }' "$report" | LC_ALL=C sort -u)
diff -u \
    <(printf '%s\n' "$negative_inventory") \
    <(awk -F'\t' '
        !/^#/ && !($1 == "path" && $2 == "variant") && $5 == "parse" { print $1 }
    ' "$report" | LC_ALL=C sort -u)

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
        echo "error: refusing to bless a non-green optional-chaining vector" >&2
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
    mv -- "$baseline_tmp" "$baseline"
    printf 'Optional chaining baseline blessed: %s/%s focused variants match QuickJS\n' \
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
    echo "error: optional-chaining Test262 classified vector drifted" >&2
    awk -F'\t' '
        !/^#/ && !($1 == "path" && $2 == "variant") && $7 != "pass" {
            print $1 "\t" $2 "\t" $7 "\t" $8 "\t" $9 "\t" $10
        }
    ' "$report" >&2
    exit 1
fi

printf 'Optional chaining Test262 gate is exact: Oxide %s/%s, QuickJS %s/%s\n' \
    "$actual_passes" "$actual_variants" \
    "$(read_value quickjs_variants)" "$(read_value variants)"
