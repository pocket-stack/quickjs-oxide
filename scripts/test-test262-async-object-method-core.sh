#!/usr/bin/env bash
# Reproduce the R3ac ordinary async object-method Test262 gate.

set -euo pipefail
export TZ=America/Los_Angeles

script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
root=$(CDPATH= cd -- "$script_dir/.." && pwd)
baseline=tests/test262-async-object-method-core-baseline.txt
manifest=tests/test262-async-object-method-core.txt
admission_profile=tests/test262-async-object-method-core.conf
exclusions=tests/test262-async-object-method-core-exclusions.tsv
global_profile=compat/test262-oxide.conf
report=target/test262-async-object-method-core.tsv
json_report=target/test262-async-object-method-core.jsonl
quickjs_log=target/test262-async-object-method-core-quickjs.log
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

profile_section() {
    local section=$1
    awk -v section="[$section]" '
        $0 == section { inside=1; next }
        /^\[/ { inside=0 }
        inside && NF && $1 !~ /^#/ { print }
    ' "$admission_profile"
}

inventory_count() {
    awk 'NF { count++ } END { print count + 0 }' "$1"
}

expect_value() {
    local key=$1 expected=$2
    if [[ "$(read_value "$key")" != "$expected" ]]; then
        echo "error: async object-method baseline $key drifted" >&2
        exit 1
    fi
}

verify_inventory() {
    local name=$1 inventory=$2
    if [[ "$(inventory_count "$inventory")" != "$(read_value "${name}_paths")" \
        || "$(sha256_file "$inventory")" != "$(read_value "${name}_sha256")" ]]; then
        echo "error: async object-method $name inventory drifted" >&2
        exit 1
    fi
}

verify_quickjs_oracle() {
    local runner=$source_dir/run-test262 test_path
    local -a files=()
    [[ -x "$runner" ]] || "${MAKE:-make}" -C "$source_dir" run-test262 >&2
    while IFS= read -r test_path; do
        files+=("test262/$test_path")
    done < "$candidate"

    # QuickJS's default mode runs one representative sloppy/strict variant
    # per candidate path, including explicitly classified adjacencies. Oxide's
    # ordinary-method gate below runs every admitted metadata-selected mode.
    if ! (cd -- "$source_dir" \
        && ./run-test262 -m -c test262.conf -f "${files[@]}") \
        >"$quickjs_log" 2>&1; then
        cat "$quickjs_log" >&2
        echo "error: pinned QuickJS could not execute the async object-method cohort" >&2
        exit 1
    fi
    if grep -Eq '(^|[[:space:]])FAILED($|[[:space:]])' "$quickjs_log" \
        || ! grep -Fq \
            "Average memory statistics for $(read_value quickjs_passes) tests:" \
            "$quickjs_log"; then
        cat "$quickjs_log" >&2
        echo "error: pinned QuickJS no longer passes the async object-method cohort" >&2
        exit 1
    fi
}

cd -- "$root"
suite=$("$script_dir/prepare-test262.sh")
source_dir=$(dirname -- "$suite")
tmp_dir=$(mktemp -d "${TMPDIR:-/tmp}/quickjs-oxide-r3ac.XXXXXX")
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

touch "$positive" "$negative" "$async_paths" "$sync_paths" \
    "$double_mode" "$no_strict" "$only_strict" "$variant_keys_raw" \
    "$sloppy_paths" "$strict_paths" "$feature_occurrences" \
    "$include_occurrences" "$flag_occurrences"

expect_value quickjs 2026-06-04
expect_value test262 5c8206929d81b2d3d727ca6aac56c18358c8d790
expect_value test262_patch_sha256 f4b23b04641d438df0826fb17d7a5db276af2bdb085b42cc09aa8d50e0da9ba3
expect_value test262_config_sha256 79c64748ff1182baf5433d0a8378e3666738a785d02faf71f0d459ed42ae897b
expect_value test262_metadata_sha256 a37219960819e56a5c5c1723d31d6a33095c778bf5347385187fde96f927a06a
expect_value global_oxide_profile_sha256 6a4d3dc37da05f6e63d7b8564483159c383ed66c665a2b5530624e628f73b908
expect_value schema test262-canonical-classified-v2
expect_value mode both
expect_value timeout_ms 30000
expect_value candidate_paths 49
expect_value candidate_function_to_string_paths 2
expect_value candidate_complex_parameters_paths 3
expect_value candidate_eval_paths 1
expect_value candidate_forbidden_extensions_paths 5
expect_value candidate_private_name_paths 2
expect_value candidate_proxy_paths 1
expect_value candidate_async_generator_paths 6
expect_value excluded_paths 7
expect_value excluded_proxy_paths 1
expect_value excluded_async_generator_paths 6
expect_value admitted_function_to_string_paths 1
expect_value admitted_complex_parameters_paths 0
expect_value admitted_eval_paths 1
expect_value admitted_forbidden_extensions_paths 5
expect_value admitted_private_name_paths 2
expect_value admitted_proxy_paths 0
expect_value admitted_async_generator_paths 0
expect_value paths 42
expect_value quickjs_passes 49
expect_value positive_paths 25
expect_value negative_paths 17
expect_value async_paths 23
expect_value sync_paths 19
expect_value double_mode_paths 34
expect_value no_strict_paths 6
expect_value only_strict_paths 2
expect_value variants 76
expect_value sloppy_variants 40
expect_value strict_variants 36
expect_value features 6
expect_value includes 1
expect_value flags 4
expect_value runnable 76
expect_value passes 76
expect_value failures 0
expect_value unsupported 0
expect_value skipped 0

if [[ "$(sha256_file "$global_profile")" != "$(read_value global_oxide_profile_sha256)" \
    || "$(sha256_file "$admission_profile")" != "$(read_value oxide_profile_sha256)" \
    || "$(sha256_file "$exclusions")" != "$(read_value exclusions_file_sha256)" ]]; then
    echo "error: async object-method pinned profile or exclusions drifted" >&2
    exit 1
fi
if grep -Fq '[execution]' "$global_profile"; then
    echo "error: global Test262 profile must remain fail-closed for async execution" >&2
    exit 1
fi

git -C "$suite" ls-files \
    'test/built-ins/Function/prototype/toString/async-method-object.js' \
    'test/built-ins/Function/prototype/toString/proxy-async-method-definition.js' \
    'test/language/expressions/object/method-definition/async-*.js' \
    'test/language/expressions/object/method-definition/forbidden-ext/b1/async-meth-*.js' \
    'test/language/expressions/object/method-definition/forbidden-ext/b2/async-meth-*.js' \
    'test/language/expressions/object/method-definition/early-errors-object-async-method-duplicate-parameters.js' \
    'test/language/expressions/object/method-definition/early-errors-object-method-arguments-in-formal-parameters.js' \
    'test/language/expressions/object/method-definition/early-errors-object-method-async-lineterminator.js' \
    'test/language/expressions/object/method-definition/early-errors-object-method-await-in-formals-default.js' \
    'test/language/expressions/object/method-definition/early-errors-object-method-await-in-formals.js' \
    'test/language/expressions/object/method-definition/early-errors-object-method-body-contains-super-call.js' \
    'test/language/expressions/object/method-definition/early-errors-object-method-eval-in-formal-parameters.js' \
    'test/language/expressions/object/method-definition/early-errors-object-method-formals-contains-super-call.js' \
    'test/language/expressions/object/method-definition/object-method-returns-promise.js' \
    'test/language/expressions/object/method-definition/private-name-early-error-async-fn-inside-class.js' \
    'test/language/expressions/object/method-definition/private-name-early-error-async-fn.js' \
    | awk '!/\/async-gen-/' \
    | LC_ALL=C sort -u > "$candidate"
LC_ALL=C sort -c "$candidate"
if [[ "$(inventory_count "$candidate")" != "$(read_value candidate_paths)" \
    || "$(sha256_file "$candidate")" != "$(read_value candidate_sha256)" \
    || "$(awk '/Function\/prototype\/toString/ { count++ } END { print count + 0 }' "$candidate")" \
        != "$(read_value candidate_function_to_string_paths)" \
    || "$(awk '
        /(array|object)-destructuring-param-strict-body\.js$/ \
            || /rest-param-strict-body\.js$/ { count++ }
        END { print count + 0 }
    ' "$candidate")" != "$(read_value candidate_complex_parameters_paths)" \
    || "$(awk '/eval-var-scope-syntax-err\.js$/ { count++ } END { print count + 0 }' "$candidate")" \
        != "$(read_value candidate_eval_paths)" \
    || "$(awk 'index($0, "/forbidden-ext/") { count++ } END { print count + 0 }' "$candidate")" \
        != "$(read_value candidate_forbidden_extensions_paths)" \
    || "$(awk '/private-name-early-error/ { count++ } END { print count + 0 }' "$candidate")" \
        != "$(read_value candidate_private_name_paths)" \
    || "$(awk '/proxy-/ { count++ } END { print count + 0 }' "$candidate")" \
        != "$(read_value candidate_proxy_paths)" \
    || "$(awk '
        /async-meth-array-destructuring-param-strict-body\.js$/ \
            || /async-meth-dflt-params-(duplicates|rest)\.js$/ \
            || /async-meth-object-destructuring-param-strict-body\.js$/ \
            || /async-meth-rest-param-strict-body\.js$/ \
            || /async-meth-rest-params-trailing-comma-early-error\.js$/ { count++ }
        END { print count + 0 }
    ' "$candidate")" != "$(read_value candidate_async_generator_paths)" ]]; then
    echo "error: async object-method candidate universe drifted" >&2
    exit 1
fi

if awk -F'\t' \
    'NF != 2 || $1 == "" || $2 == "" { print NR ":" $0; bad=1 } END { exit bad ? 0 : 1 }' \
    "$exclusions" >&2; then
    echo "error: async object-method exclusions must have two populated TSV columns" >&2
    exit 1
fi
awk -F'\t' '{ print $2 }' "$exclusions" > "$excluded_paths"
LC_ALL=C sort -c "$excluded_paths"
if [[ "$(inventory_count "$excluded_paths")" != "$(read_value excluded_paths)" \
    || "$(LC_ALL=C sort -u "$excluded_paths" | inventory_count /dev/stdin)" \
        != "$(read_value excluded_paths)" \
    || "$(sha256_file "$excluded_paths")" != "$(read_value excluded_paths_sha256)" \
    || "$(awk -F'\t' '$1 == "proxy" { count++ } END { print count + 0 }' "$exclusions")" \
        != "$(read_value excluded_proxy_paths)" \
    || "$(awk -F'\t' '$1 == "async_generator" { count++ } END { print count + 0 }' "$exclusions")" \
        != "$(read_value excluded_async_generator_paths)" \
    || "$(awk -F'\t' \
        '$1 != "proxy" && $1 != "async_generator" { count++ } \
        END { print count + 0 }' "$exclusions")" \
        != "0" ]]; then
    echo "error: async object-method exclusion ledger drifted" >&2
    exit 1
fi
if [[ -n "$(comm -23 "$excluded_paths" "$candidate")" ]]; then
    echo "error: async object-method exclusion escaped the candidate universe" >&2
    exit 1
fi

comm -23 "$candidate" "$excluded_paths" > "$derived_manifest"
diff -u "$manifest" "$derived_manifest"
LC_ALL=C sort -u "$manifest" "$excluded_paths" > "$partition_union"
diff -u "$candidate" "$partition_union"
if [[ -n "$(comm -12 "$manifest" "$excluded_paths")" ]]; then
    echo "error: async object-method manifest and exclusions overlap" >&2
    exit 1
fi
LC_ALL=C sort -c "$manifest"
if [[ "$(inventory_count "$manifest")" != "$(read_value paths)" \
    || "$(sha256_file "$manifest")" != "$(read_value manifest_sha256)" \
    || "$(sha256_file "$manifest")" != "$(read_value manifest_file_sha256)" \
    || "$(awk '/Function\/prototype\/toString/ { count++ } END { print count + 0 }' "$manifest")" \
        != "$(read_value admitted_function_to_string_paths)" \
    || "$(awk '
        /(array|object)-destructuring-param-strict-body\.js$/ \
            || /rest-param-strict-body\.js$/ { count++ }
        END { print count + 0 }
    ' "$manifest")" != "$(read_value admitted_complex_parameters_paths)" \
    || "$(awk '/eval-var-scope-syntax-err\.js$/ { count++ } END { print count + 0 }' "$manifest")" \
        != "$(read_value admitted_eval_paths)" \
    || "$(awk 'index($0, "/forbidden-ext/") { count++ } END { print count + 0 }' "$manifest")" \
        != "$(read_value admitted_forbidden_extensions_paths)" \
    || "$(awk '/private-name-early-error/ { count++ } END { print count + 0 }' "$manifest")" \
        != "$(read_value admitted_private_name_paths)" \
    || "$(awk '/proxy-/ { count++ } END { print count + 0 }' "$manifest")" \
        != "$(read_value admitted_proxy_paths)" \
    || "$(awk '
        /async-meth-array-destructuring-param-strict-body\.js$/ \
            || /async-meth-dflt-params-(duplicates|rest)\.js$/ \
            || /async-meth-object-destructuring-param-strict-body\.js$/ \
            || /async-meth-rest-param-strict-body\.js$/ \
            || /async-meth-rest-params-trailing-comma-early-error\.js$/ { count++ }
        END { print count + 0 }
    ' "$manifest")" != "$(read_value admitted_async_generator_paths)" ]]; then
    echo "error: async object-method manifest drifted" >&2
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
    -v expected="$(read_value paths)" \
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
        if (seen != expected) {
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
    echo "error: async object-method metadata composition drifted" >&2
    exit 1
fi

diff -u <(profile_section features | LC_ALL=C sort) "$feature_inventory"
diff -u <(profile_section audited-negative-tests | LC_ALL=C sort) "$negative"
[[ "$(profile_section execution)" == "async=true" ]] \
    || { echo "error: async object-method profile must opt into only the async host" >&2; exit 1; }

verify_quickjs_oracle
if "$check_only"; then
    printf 'async object-method inputs verified: %s candidates - %s explicit exclusions = %s paths; QuickJS %s passes all candidates; Oxide gate covers %s variants\n' \
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
    echo "error: async object-method report metadata drifted" >&2
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
    echo "error: async object-method all-pass vector drifted" >&2
    awk -F'\t' '
        !/^#/ && !($1 == "path" && $2 == "variant") && $7 != "pass" {
            print $1 "\t" $2 "\t" $7 "\t" $8 "\t" $9 "\t" $10
        }
    ' "$report" >&2
    exit 1
fi

printf 'async object-method Test262 gate passes: %s/%s variants across %s audited paths\n' \
    "$actual_passes" "$actual_variants" "$(read_value paths)"
