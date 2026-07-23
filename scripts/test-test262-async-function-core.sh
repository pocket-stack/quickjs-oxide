#!/usr/bin/env bash
# Reproduce the R3z ordinary async-function and await core Test262 gate.

set -euo pipefail
export TZ=America/Los_Angeles

script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
root=$(CDPATH= cd -- "$script_dir/.." && pwd)
baseline=tests/test262-async-function-core-baseline.txt
manifest=tests/test262-async-function-core.txt
admission_profile=tests/test262-async-function-core.conf
exclusions=tests/test262-async-function-core-exclusions.tsv
global_profile=compat/test262-oxide.conf
report=target/test262-async-function-core.tsv
json_report=target/test262-async-function-core.jsonl
quickjs_log=target/test262-async-function-core-quickjs.log
workers=${TEST262_WORKERS:-8}

usage() {
    printf 'usage: %s [--check]\n' "${0##*/}"
    printf '  --check  verify frozen inputs, exhaustive metadata, and pinned QuickJS; skip Oxide\n'
}

check_only=false
case ${1-} in
    "") ;;
    --check) check_only=true ;;
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

profile_section() {
    local section=$1 profile=${2:-$admission_profile}
    awk -v section="[$section]" '
        $0 == section { inside=1; next }
        /^\[/ { inside=0 }
        inside && NF && $1 !~ /^#/ { print }
    ' "$profile"
}

inventory_count() {
    awk 'NF { count++ } END { print count + 0 }' "$1"
}

verify_inventory() {
    local name=$1 inventory=$2
    if [[ "$(inventory_count "$inventory")" != "$(read_value "${name}_paths")" \
        || "$(sha256_file "$inventory")" != "$(read_value "${name}_sha256")" ]]; then
        echo "error: async-function core $name inventory drifted" >&2
        exit 1
    fi
}

verify_quickjs_oracle() {
    local runner=$source_dir/run-test262 test_path
    local -a files=()
    [[ -x "$runner" ]] || "${MAKE:-make}" -C "$source_dir" run-test262 >&2
    while IFS= read -r test_path; do
        files+=("test262/$test_path")
    done < "$manifest"

    # The pinned config's default mode executes exactly one representative
    # sloppy/strict variant per path; Oxide's canonical gate below runs both.
    if ! (cd -- "$source_dir" \
        && ./run-test262 -m -c test262.conf -f "${files[@]}") \
        >"$quickjs_log" 2>&1; then
        cat "$quickjs_log" >&2
        echo "error: pinned QuickJS could not execute the async-function core cohort" >&2
        exit 1
    fi
    if grep -Eq '(^|[[:space:]])FAILED($|[[:space:]])' "$quickjs_log" \
        || ! grep -Fq \
            "Average memory statistics for $(read_value quickjs_passes) tests:" \
            "$quickjs_log"; then
        cat "$quickjs_log" >&2
        echo "error: pinned QuickJS no longer passes the async-function core cohort" >&2
        exit 1
    fi
}

cd -- "$root"
suite=$("$script_dir/prepare-test262.sh")
source_dir=$(dirname -- "$suite")
tmp_dir=$(mktemp -d "${TMPDIR:-/tmp}/quickjs-oxide-r3z.XXXXXX")
trap 'rm -rf -- "$tmp_dir"' EXIT HUP INT TERM

candidate=$tmp_dir/candidate.txt
excluded_paths=$tmp_dir/excluded-paths.txt
derived_manifest=$tmp_dir/manifest.txt
partition_union=$tmp_dir/partition-union.txt
metadata_records=$tmp_dir/metadata.records
metadata_tsv=$tmp_dir/metadata.tsv
positive=$tmp_dir/positive.txt
negative=$tmp_dir/negative.txt
async_paths=$tmp_dir/async.txt
sync_paths=$tmp_dir/sync.txt
double_mode=$tmp_dir/double-mode.txt
no_strict=$tmp_dir/no-strict.txt
only_strict=$tmp_dir/only-strict.txt
variant_keys_raw=$tmp_dir/variant-keys.raw
variant_keys=$tmp_dir/variant-keys.txt
sloppy_paths=$tmp_dir/sloppy.txt
strict_paths=$tmp_dir/strict.txt
feature_occurrences=$tmp_dir/features.raw
include_occurrences=$tmp_dir/includes.raw
flag_occurrences=$tmp_dir/flags.raw
feature_inventory=$tmp_dir/features.txt
include_inventory=$tmp_dir/includes.txt
flag_inventory=$tmp_dir/flags.txt

if [[ "$(read_value quickjs)" != "2026-06-04" \
    || "$(read_value test262)" != "5c8206929d81b2d3d727ca6aac56c18358c8d790" \
    || "$(read_value test262_patch_sha256)" != "f4b23b04641d438df0826fb17d7a5db276af2bdb085b42cc09aa8d50e0da9ba3" \
    || "$(read_value test262_config_sha256)" != "79c64748ff1182baf5433d0a8378e3666738a785d02faf71f0d459ed42ae897b" \
    || "$(read_value test262_metadata_sha256)" != "a37219960819e56a5c5c1723d31d6a33095c778bf5347385187fde96f927a06a" \
    || "$(read_value global_oxide_profile_sha256)" != "6a4d3dc37da05f6e63d7b8564483159c383ed66c665a2b5530624e628f73b908" \
    || "$(read_value oxide_profile_sha256)" != "05634144cdc2e64874ffda721b429181ac8b7a8f82b1ba253f2b8d8a29a4332e" \
    || "$(read_value schema)" != "test262-canonical-classified-v2" \
    || "$(read_value mode)" != "both" \
    || "$(read_value timeout_ms)" != "30000" \
    || "$(read_value candidate_builtins_paths)" != "18" \
    || "$(read_value candidate_expression_async_function_paths)" != "93" \
    || "$(read_value candidate_expression_await_paths)" != "22" \
    || "$(read_value candidate_statement_async_function_paths)" != "74" \
    || "$(read_value candidate_paths)" != "207" \
    || "$(read_value excluded_paths)" != "65" \
    || "$(read_value complex_parameters_paths)" != "40" \
    || "$(read_value eval_or_with_paths)" != "11" \
    || "$(read_value async_arrow_paths)" != "10" \
    || "$(read_value async_generator_or_for_await_paths)" != "2" \
    || "$(read_value host_or_cross_realm_paths)" != "2" \
    || "$(read_value paths)" != "142" \
    || "$(read_value quickjs_passes)" != "142" \
    || "$(read_value positive_paths)" != "95" \
    || "$(read_value negative_paths)" != "47" \
    || "$(read_value async_paths)" != "65" \
    || "$(read_value sync_paths)" != "77" \
    || "$(read_value double_mode_paths)" != "117" \
    || "$(read_value no_strict_paths)" != "17" \
    || "$(read_value only_strict_paths)" != "8" \
    || "$(read_value variants)" != "259" \
    || "$(read_value sloppy_variants)" != "134" \
    || "$(read_value strict_variants)" != "125" \
    || "$(read_value features)" != "4" \
    || "$(read_value includes)" != "3" \
    || "$(read_value flags)" != "4" \
    || "$(read_value runnable)" != "259" \
    || "$(read_value passes)" != "259" \
    || "$(read_value failures)" != "0" \
    || "$(read_value unsupported)" != "0" \
    || "$(read_value skipped)" != "0" ]]; then
    echo "error: async-function core baseline identity drifted" >&2
    exit 1
fi

[[ "$(sha256_file "$global_profile")" == "$(read_value global_oxide_profile_sha256)" \
    && "$(sha256_file "$admission_profile")" == "$(read_value oxide_profile_sha256)" \
    && "$(sha256_file "$exclusions")" == "$(read_value exclusions_file_sha256)" ]] \
    || { echo "error: async-function core pinned profile or exclusions drifted" >&2; exit 1; }
if grep -Fq '[execution]' "$global_profile"; then
    echo "error: global Test262 profile must remain fail-closed for async execution" >&2
    exit 1
fi

git -C "$suite" ls-files \
    'test/built-ins/AsyncFunction/*.js' \
    'test/language/expressions/async-function/*.js' \
    'test/language/expressions/await/*.js' \
    'test/language/statements/async-function/*.js' \
    | LC_ALL=C sort -u > "$candidate"
LC_ALL=C sort -c "$candidate"
if [[ "$(inventory_count "$candidate")" != "$(read_value candidate_paths)" \
    || "$(sha256_file "$candidate")" != "$(read_value candidate_sha256)" \
    || "$(awk 'index($0, "test/built-ins/AsyncFunction/") == 1 { count++ } END { print count + 0 }' "$candidate")" \
        != "$(read_value candidate_builtins_paths)" \
    || "$(awk 'index($0, "test/language/expressions/async-function/") == 1 { count++ } END { print count + 0 }' "$candidate")" \
        != "$(read_value candidate_expression_async_function_paths)" \
    || "$(awk 'index($0, "test/language/expressions/await/") == 1 { count++ } END { print count + 0 }' "$candidate")" \
        != "$(read_value candidate_expression_await_paths)" \
    || "$(awk 'index($0, "test/language/statements/async-function/") == 1 { count++ } END { print count + 0 }' "$candidate")" \
        != "$(read_value candidate_statement_async_function_paths)" ]]; then
    echo "error: async-function core candidate universe drifted" >&2
    exit 1
fi

if awk -F'\t' 'NF != 2 || $1 == "" || $2 == "" { print NR ":" $0; bad=1 } END { exit bad ? 0 : 1 }' \
    "$exclusions" >&2; then
    echo "error: async-function core exclusions must have exactly two populated TSV columns" >&2
    exit 1
fi
awk -F'\t' '{ print $2 }' "$exclusions" > "$excluded_paths"
LC_ALL=C sort -c "$excluded_paths"
if [[ "$(inventory_count "$excluded_paths")" != "$(read_value excluded_paths)" \
    || "$(LC_ALL=C sort -u "$excluded_paths" | inventory_count /dev/stdin)" \
        != "$(read_value excluded_paths)" \
    || "$(sha256_file "$excluded_paths")" != "$(read_value excluded_paths_sha256)" ]]; then
    echo "error: async-function core excluded-path inventory drifted" >&2
    exit 1
fi
if [[ "$(awk -F'\t' '$1 == "complex_parameters" { count++ } END { print count + 0 }' "$exclusions")" \
        != "$(read_value complex_parameters_paths)" \
    || "$(awk -F'\t' '$1 == "eval_or_with" { count++ } END { print count + 0 }' "$exclusions")" \
        != "$(read_value eval_or_with_paths)" \
    || "$(awk -F'\t' '$1 == "async_arrow" { count++ } END { print count + 0 }' "$exclusions")" \
        != "$(read_value async_arrow_paths)" \
    || "$(awk -F'\t' '$1 == "async_generator_or_for_await" { count++ } END { print count + 0 }' "$exclusions")" \
        != "$(read_value async_generator_or_for_await_paths)" \
    || "$(awk -F'\t' '$1 == "host_or_cross_realm" { count++ } END { print count + 0 }' "$exclusions")" \
        != "$(read_value host_or_cross_realm_paths)" \
    || "$(awk -F'\t' \
        '$1 != "complex_parameters" && $1 != "eval_or_with" && $1 != "async_arrow" \
            && $1 != "async_generator_or_for_await" && $1 != "host_or_cross_realm" { count++ } \
        END { print count + 0 }' "$exclusions")" != "0" ]]; then
    echo "error: async-function core exclusion categories drifted" >&2
    exit 1
fi

if [[ -n "$(comm -23 "$excluded_paths" "$candidate")" ]]; then
    echo "error: async-function core exclusion escaped the candidate universe" >&2
    exit 1
fi
comm -23 "$candidate" "$excluded_paths" > "$derived_manifest"
diff -u "$manifest" "$derived_manifest"
LC_ALL=C sort -u "$manifest" "$excluded_paths" > "$partition_union"
diff -u "$candidate" "$partition_union"
if [[ -n "$(comm -12 "$manifest" "$excluded_paths")" ]]; then
    echo "error: async-function core manifest and exclusions overlap" >&2
    exit 1
fi
LC_ALL=C sort -c "$manifest"
if [[ "$(inventory_count "$manifest")" != "$(read_value paths)" \
    || "$(sha256_file "$manifest")" != "$(read_value manifest_sha256)" \
    || "$(sha256_file "$manifest")" != "$(read_value manifest_file_sha256)" ]]; then
    echo "error: async-function core manifest drifted" >&2
    exit 1
fi

cargo run --locked --release --quiet --bin run-test262 -- \
    --suite "$suite" \
    --validate-metadata "$metadata_records"
if [[ "$(sha256_file "$metadata_records")" != "$(read_value test262_metadata_sha256)" ]]; then
    echo "error: pinned exhaustive Test262 metadata fingerprint drifted" >&2
    exit 1
fi
tr '\0' '\t' < "$metadata_records" > "$metadata_tsv"

awk -F'\t' \
    -v positive="$positive" \
    -v negative="$negative" \
    -v async_paths="$async_paths" \
    -v sync_paths="$sync_paths" \
    -v double_mode="$double_mode" \
    -v no_strict="$no_strict" \
    -v only_strict="$only_strict" \
    -v variant_keys="$variant_keys_raw" \
    -v sloppy_paths="$sloppy_paths" \
    -v strict_paths="$strict_paths" \
    -v feature_occurrences="$feature_occurrences" \
    -v include_occurrences="$include_occurrences" \
    -v flag_occurrences="$flag_occurrences" '
    NR == FNR { selected[$1]=1; next }
    function has(list, value) {
        return index("," list ",", "," value ",") != 0
    }
    $1 in selected {
        seen++
        if ($5 == "") {
            print $1 > positive
        } else {
            if ($5 != "parse" || $6 != "SyntaxError") {
                print "bad negative provenance: " $1 > "/dev/stderr"
                exit 2
            }
            print $1 > negative
        }

        if (has($3, "async")) print $1 > async_paths
        else print $1 > sync_paths

        if (has($3, "module") || has($3, "raw") ||
            (has($3, "noStrict") && has($3, "onlyStrict"))) {
            print "unsupported mode metadata: " $1 > "/dev/stderr"
            exit 2
        } else if (has($3, "noStrict")) {
            print $1 > no_strict
            print $1 "\tsloppy" > variant_keys
            print $1 > sloppy_paths
        } else if (has($3, "onlyStrict")) {
            print $1 > only_strict
            print $1 "\tstrict" > variant_keys
            print $1 > strict_paths
        } else {
            print $1 > double_mode
            print $1 "\tsloppy" > variant_keys
            print $1 "\tstrict" > variant_keys
            print $1 > sloppy_paths
            print $1 > strict_paths
        }

        count=split($4, values, ",")
        for (i=1; i<=count; i++) {
            if (values[i] != "") print values[i] > feature_occurrences
        }
        count=split($2, values, ",")
        for (i=1; i<=count; i++) {
            if (values[i] != "") print values[i] > include_occurrences
        }
        count=split($3, values, ",")
        for (i=1; i<=count; i++) {
            if (values[i] != "") print values[i] > flag_occurrences
        }
    }
    END {
        if (seen != 142) {
            print "selected metadata rows: " seen > "/dev/stderr"
            exit 2
        }
    }
' "$manifest" "$metadata_tsv"

LC_ALL=C sort "$variant_keys_raw" > "$variant_keys"
LC_ALL=C sort -u "$feature_occurrences" > "$feature_inventory"
LC_ALL=C sort -u "$include_occurrences" > "$include_inventory"
LC_ALL=C sort -u "$flag_occurrences" > "$flag_inventory"

verify_inventory positive "$positive"
verify_inventory negative "$negative"
verify_inventory async "$async_paths"
verify_inventory sync "$sync_paths"
verify_inventory double_mode "$double_mode"
verify_inventory no_strict "$no_strict"
verify_inventory only_strict "$only_strict"
if [[ "$(inventory_count "$variant_keys")" != "$(read_value variants)" \
    || "$(sha256_file "$variant_keys")" != "$(read_value keys_sha256)" \
    || "$(inventory_count "$sloppy_paths")" != "$(read_value sloppy_variants)" \
    || "$(inventory_count "$strict_paths")" != "$(read_value strict_variants)" \
    || "$(inventory_count "$feature_inventory")" != "$(read_value features)" \
    || "$(sha256_file "$feature_inventory")" != "$(read_value features_sha256)" \
    || "$(inventory_count "$include_inventory")" != "$(read_value includes)" \
    || "$(sha256_file "$include_inventory")" != "$(read_value includes_sha256)" \
    || "$(inventory_count "$flag_inventory")" != "$(read_value flags)" \
    || "$(sha256_file "$flag_inventory")" != "$(read_value flags_sha256)" ]]; then
    echo "error: async-function core metadata composition drifted" >&2
    exit 1
fi

diff -u <(profile_section features | LC_ALL=C sort) "$feature_inventory"
diff -u <(profile_section audited-negative-tests | LC_ALL=C sort) "$negative"
[[ "$(profile_section execution)" == "async=true" ]] \
    || { echo "error: async-function core profile must opt into only the async host" >&2; exit 1; }

verify_quickjs_oracle
if "$check_only"; then
    printf 'async-function core inputs verified: %s candidates - %s explicit exclusions = %s paths; QuickJS %s passes all; Oxide gate covers %s variants\n' \
        "$(read_value candidate_paths)" \
        "$(read_value excluded_paths)" \
        "$(read_value paths)" \
        "$(read_value quickjs)" \
        "$(read_value variants)"
    exit 0
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

actual_variants=$(awk -F'\t' '
    !/^#/ && !($1 == "path" && $2 == "variant") { count++ }
    END { print count + 0 }
' "$report")
execution_line=$(printf '%s\n' "$run_output" | awk '
    /^execution: runnable=/ { print; found=1 }
    END { if (!found) exit 1 }
')
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
    echo "error: async-function core report metadata drifted" >&2
    exit 1
fi

diff -u "$variant_keys" <(awk -F'\t' '
    !/^#/ && !($1 == "path" && $2 == "variant") { print $1 "\t" $2 }
' "$report" | LC_ALL=C sort)
diff -u "$manifest" <(awk -F'\t' '
    !/^#/ && !($1 == "path" && $2 == "variant") { print $1 }
' "$report" | LC_ALL=C sort -u)
diff -u "$feature_inventory" <(awk -F'\t' '
    !/^#/ && !($1 == "path" && $2 == "variant") {
        count=split($4, features, ",")
        for (i=1; i<=count; i++) if (features[i] != "") print features[i]
    }
' "$report" | LC_ALL=C sort -u)

actual_keys=$(awk -F'\t' '
    !/^#/ && !($1 == "path" && $2 == "variant") { print $1 "\t" $2 }
' "$report" | LC_ALL=C sort | sha256_stream)
actual_passes=$(awk -F'\t' '
    !/^#/ && !($1 == "path" && $2 == "variant") && $7 == "pass" { count++ }
    END { print count + 0 }
' "$report")
actual_unsupported=$(awk -F'\t' '
    !/^#/ && !($1 == "path" && $2 == "variant") && $7 ~ /^unsupported-/ { count++ }
    END { print count + 0 }
' "$report")
actual_skipped=$(awk -F'\t' '
    !/^#/ && !($1 == "path" && $2 == "variant") && $7 ~ /^skipped-/ { count++ }
    END { print count + 0 }
' "$report")
actual_failures=$((actual_variants - actual_passes - actual_unsupported - actual_skipped))
actual_nonpass=$(awk -F'\t' '
    !/^#/ && !($1 == "path" && $2 == "variant") && $7 != "pass" {
        print $1 "\t" $2 "\t" $7 "\t" $8 "\t" $9 "\t" $10
    }
' "$report" | sha256_stream)
actual_summary=$(tail -n 1 "$report" | sed 's/^# summary //')
runner_summary=$(printf '%s\n' "$run_output" | awk '
    /^Test262: total=/ { print; found=1 }
    END { if (!found) exit 1 }
')
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
    echo "error: async-function core all-pass vector drifted" >&2
    awk -F'\t' '
        !/^#/ && !($1 == "path" && $2 == "variant") && $7 != "pass" {
            print $1 "\t" $2 "\t" $7 "\t" $8 "\t" $9 "\t" $10
        }
    ' "$report" >&2
    exit 1
fi

printf 'async-function core Test262 gate passes: %s/%s variants across %s audited paths\n' \
    "$actual_passes" "$actual_variants" "$(read_value paths)"
