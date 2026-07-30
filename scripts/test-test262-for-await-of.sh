#!/usr/bin/env bash
# Reproduce the R3bk refreshed for-await-of Test262 gate.

set -euo pipefail
export TZ=America/Los_Angeles

script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
root=$(CDPATH= cd -- "$script_dir/.." && pwd)
baseline=compat/test262-for-await-of-baseline.txt
admission_profile=compat/test262-for-await-of.conf
exclusions=compat/test262-for-await-of-exclusions.tsv
global_profile=compat/test262-oxide.conf
report=target/test262-for-await-of.tsv
json_report=target/test262-for-await-of.jsonl
quickjs_log=target/test262-for-await-of-quickjs.log
candidate_quickjs_log=target/test262-for-await-of-candidate-quickjs.log
workers=${TEST262_WORKERS:-8}
ledgers=(
    tests/test262-async-generator-core-exclusions.tsv
    tests/test262-async-generator-object-method-core-exclusions.tsv
    tests/test262-async-generator-class-method-core-exclusions.tsv
    tests/test262-async-generator-private-class-method-core-exclusions.tsv
)
ledger_hash_keys=(
    async_generator_core_exclusions
    async_generator_object_method_core_exclusions
    async_generator_class_method_core_exclusions
    async_generator_private_class_method_core_exclusions
)
shape_names=(
    inherited_function
    inherited_object_method
    inherited_public_class
    inherited_private_class
)
ledger_for_await_counts=(6 2 8 8)

usage() {
    printf 'usage: %s [--check]\n' "${0##*/}"
    printf '  --check  verify frozen inputs, metadata, partitions, and pinned QuickJS; skip Oxide\n'
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

inventory_count() {
    awk 'NF { count++ } END { print count + 0 }' "$1"
}

profile_section() {
    local section=$1 profile=${2:-$admission_profile}
    awk -v section="[$section]" '
        $0 == section { inside=1; next }
        /^\[/ { inside=0 }
        inside && NF && $1 !~ /^#/ { print }
    ' "$profile"
}

expect_value() {
    local key=$1 expected=$2
    if [[ "$(read_value "$key")" != "$expected" ]]; then
        echo "error: for-await-of baseline $key drifted" >&2
        exit 1
    fi
}

verify_inventory() {
    local prefix=$1 inventory=$2
    if [[ "$(inventory_count "$inventory")" != "$(read_value "${prefix}_paths")" \
        || "$(sha256_file "$inventory")" != "$(read_value "${prefix}_sha256")" ]]; then
        echo "error: for-await-of $prefix inventory drifted" >&2
        exit 1
    fi
}

verify_plain_inventory() {
    local prefix=$1 inventory=$2
    if [[ "$(inventory_count "$inventory")" != "$(read_value "$prefix")" \
        || "$(sha256_file "$inventory")" != "$(read_value "${prefix}_sha256")" ]]; then
        echo "error: for-await-of $prefix inventory drifted" >&2
        exit 1
    fi
}

derive_variant_keys() {
    local selected=$1 output=$2 expected
    expected=$(inventory_count "$selected")
    awk -F'\t' -v expected="$expected" '
        NR == FNR { selected[$1]=1; next }
        function has(list, value) {
            return index("," list ",", "," value ",") != 0
        }
        $1 in selected {
            seen++
            if ((has($3, "noStrict") && has($3, "onlyStrict")) ||
                (has($3, "module") && (has($3, "noStrict") || has($3, "onlyStrict"))) ||
                (has($3, "raw") && has($3, "onlyStrict"))) {
                print "conflicting mode metadata: " $1 > "/dev/stderr"
                exit 2
            }
            if (has($3, "module") || has($3, "noStrict") || has($3, "raw")) {
                print $1 "\tsloppy"
            } else if (has($3, "onlyStrict")) {
                print $1 "\tstrict"
            } else {
                print $1 "\tsloppy"
                print $1 "\tstrict"
            }
        }
        END {
            if (seen != expected) {
                print "selected metadata rows: " seen > "/dev/stderr"
                exit 2
            }
        }
    ' "$selected" "$metadata_tsv" | LC_ALL=C sort > "$output"
}

verify_variant_count() {
    local inventory=$1 expected=$2 output=$3
    derive_variant_keys "$inventory" "$output"
    if [[ "$(inventory_count "$output")" != "$expected" ]]; then
        echo "error: for-await-of subgroup variants drifted: $inventory" >&2
        exit 1
    fi
}

verify_quickjs_oracle() {
    local inventory=$1 expected=$2 output_log=$3 label=$4
    local runner=$source_dir/run-test262 test_path
    local -a files=()
    [[ -x "$runner" ]] || "${MAKE:-make}" -C "$source_dir" run-test262 >&2
    while IFS= read -r test_path; do
        files+=("test262/$test_path")
    done < "$inventory"

    if ! (cd -- "$source_dir" \
        && ./run-test262 -a -m -c test262.conf -f "${files[@]}") \
        >"$output_log" 2>&1; then
        cat "$output_log" >&2
        echo "error: pinned QuickJS could not execute the $label cohort" >&2
        exit 1
    fi
    if grep -Eq '(^|[[:space:]])FAILED($|[[:space:]])' "$output_log" \
        || ! grep -Fq "Average memory statistics for $expected tests:" "$output_log"; then
        cat "$output_log" >&2
        echo "error: pinned QuickJS no longer passes the $label cohort" >&2
        exit 1
    fi
}

run_oxide() {
    local worker_count=$1 output_report=$2
    cargo run --locked --release --quiet --bin run-test262 -- \
        --suite "$suite" \
        --config "$source_dir/test262.conf" \
        --oxide-profile "$admission_profile" \
        --manifest "$manifest" \
        --report "$output_report" \
        --mode "$(read_value mode)" \
        --workers "$worker_count" \
        --timeout-ms "$(read_value timeout_ms)" \
        --allow-failures
}

cd -- "$root"
suite=$("$script_dir/prepare-test262.sh")
source_dir=$(dirname -- "$suite")
tmp_dir=$(mktemp -d "${TMPDIR:-/tmp}/quickjs-oxide-r3bk.XXXXXX")
trap 'rm -rf -- "$tmp_dir"' EXIT HUP INT TERM

metadata_records=$tmp_dir/metadata.records
metadata_tsv=$tmp_dir/metadata.tsv
related_raw=$tmp_dir/related-raw.txt
candidate=$tmp_dir/candidate.txt
candidate_main=$tmp_dir/candidate-main.txt
candidate_external=$tmp_dir/candidate-external.txt
derived_exclusions=$tmp_dir/derived-exclusions.tsv
excluded_paths=$tmp_dir/excluded-paths.txt
promoted_optional_chaining=$tmp_dir/promoted-optional-chaining.txt
promoted_optional_chaining_keys=$tmp_dir/promoted-optional-chaining-keys.txt
manifest=$tmp_dir/manifest.txt
partition_union=$tmp_dir/partition-union.txt
candidate_keys=$tmp_dir/candidate-keys.txt
excluded_keys=$tmp_dir/excluded-keys.txt
variant_keys=$tmp_dir/variant-keys.txt
manifest_main=$tmp_dir/manifest-main.txt
manifest_external=$tmp_dir/manifest-external.txt
destructuring=$tmp_dir/destructuring.txt
core_main=$tmp_dir/core-main.txt
async_from_sync=$tmp_dir/async-from-sync.txt
async_interleaving=$tmp_dir/async-interleaving.txt
staging_grammar=$tmp_dir/staging-grammar.txt
inherited_raw=$tmp_dir/inherited-raw.txt
inherited=$tmp_dir/inherited.txt
positive=$tmp_dir/positive.txt
negative=$tmp_dir/negative.txt
async_paths=$tmp_dir/async.txt
sync_paths=$tmp_dir/sync.txt
double_mode=$tmp_dir/double-mode.txt
no_strict=$tmp_dir/no-strict.txt
only_strict=$tmp_dir/only-strict.txt
feature_occurrences=$tmp_dir/features.raw
include_occurrences=$tmp_dir/includes.raw
flag_occurrences=$tmp_dir/flags.raw
feature_inventory=$tmp_dir/features.txt
include_inventory=$tmp_dir/includes.txt
flag_inventory=$tmp_dir/flags.txt
expected_features=$tmp_dir/expected-features.txt
expected_includes=$tmp_dir/expected-includes.txt
expected_flags=$tmp_dir/expected-flags.txt
expected_negative=$tmp_dir/expected-negative.txt

for output in \
    "$positive" "$negative" "$async_paths" "$sync_paths" "$double_mode" \
    "$no_strict" "$only_strict" "$variant_keys" "$feature_occurrences" \
    "$include_occurrences" "$flag_occurrences" "$inherited_raw"; do
    : > "$output"
done

expect_value quickjs 2026-06-04
expect_value test262 5c8206929d81b2d3d727ca6aac56c18358c8d790
expect_value test262_patch_sha256 f4b23b04641d438df0826fb17d7a5db276af2bdb085b42cc09aa8d50e0da9ba3
expect_value test262_config_sha256 79c64748ff1182baf5433d0a8378e3666738a785d02faf71f0d459ed42ae897b
expect_value test262_metadata_sha256 a37219960819e56a5c5c1723d31d6a33095c778bf5347385187fde96f927a06a
expect_value r3al_global_oxide_profile_sha256 fc6e8010c982bd6324b146e5f8e3ea0592aac7c03a323a8dbc8d778b4b670b23
expect_value schema test262-canonical-classified-v2
expect_value mode both
expect_value timeout_ms 30000
expect_value candidate_paths 1297
expect_value candidate_main_directory_paths 1234
expect_value candidate_external_paths 63
expect_value candidate_variants 2531
expect_value excluded_paths 32
expect_value excluded_variants 39
expect_value excluded_baseline_unsupported_erm_paths 3
expect_value excluded_module_or_dynamic_import_paths 28
expect_value excluded_host_is_html_dda_paths 1
expect_value promoted_optional_chaining_paths 1
expect_value promoted_optional_chaining_variants 2
expect_value paths 1265
expect_value main_directory_paths 1232
expect_value main_directory_variants 2427
expect_value destructuring_paths 1215
expect_value destructuring_variants 2396
expect_value core_main_directory_paths 17
expect_value core_main_directory_variants 31
expect_value external_paths 33
expect_value external_variants 65
expect_value inherited_for_await_paths 24
expect_value inherited_for_await_variants 48
expect_value inherited_function_paths 6
expect_value inherited_object_method_paths 2
expect_value inherited_public_class_paths 8
expect_value inherited_private_class_paths 8
expect_value async_from_sync_iterator_paths 5
expect_value async_from_sync_iterator_variants 10
expect_value async_interleaving_paths 1
expect_value async_interleaving_variants 2
expect_value staging_grammar_paths 2
expect_value staging_grammar_variants 3
expect_value quickjs_candidate_enabled_passes 1294
expect_value quickjs_passes 1265
expect_value positive_paths 1175
expect_value negative_paths 90
expect_value async_paths 1171
expect_value sync_paths 94
expect_value double_mode_paths 1227
expect_value no_strict_paths 31
expect_value only_strict_paths 7
expect_value variants 2492
expect_value features 13
expect_value includes 3
expect_value flags 4

if [[ "$(sha256_file "$admission_profile")" != "$(read_value oxide_profile_sha256)" \
    || "$(sha256_file "$exclusions")" != "$(read_value exclusions_file_sha256)" ]]; then
    echo "error: for-await-of focused profile or exclusion ledger drifted" >&2
    exit 1
fi
if [[ "$(profile_section execution "$global_profile")" != "async=true" ]]; then
    echo "error: global Test262 profile must admit only async execution" >&2
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

{
    git -C "$suite" ls-files 'test/**/*.js' \
        | grep -Ei 'for[-_]await|forawait'
    git -C "$suite" grep -l -E 'for[[:space:]]+await' -- 'test/**/*.js'
} | LC_ALL=C sort -u > "$related_raw"
awk -F'\t' '
    NR == FNR { executable[$1]=1; next }
    $1 in executable { print $1 }
' "$metadata_tsv" "$related_raw" | LC_ALL=C sort -u > "$candidate"
awk '/^test\/language\/statements\/for-await-of\// { print }' \
    "$candidate" > "$candidate_main"
awk '!/^test\/language\/statements\/for-await-of\// { print }' \
    "$candidate" > "$candidate_external"
verify_inventory candidate "$candidate"
verify_inventory candidate_main_directory "$candidate_main"
verify_inventory candidate_external "$candidate_external"
derive_variant_keys "$candidate" "$candidate_keys"
if [[ "$(inventory_count "$candidate_keys")" != "$(read_value candidate_variants)" \
    || "$(sha256_file "$candidate_keys")" != "$(read_value candidate_keys_sha256)" ]]; then
    echo "error: for-await-of candidate variants drifted" >&2
    exit 1
fi

if awk -F'\t' \
    'NF != 2 || $1 == "" || $2 == "" { print NR ":" $0; bad=1 } END { exit bad ? 0 : 1 }' \
    "$exclusions" >&2; then
    echo "error: for-await-of exclusions need two populated TSV columns" >&2
    exit 1
fi
if awk -F'\t' '
    $1 != "baseline_unsupported_erm" &&
        $1 != "module_or_dynamic_import" &&
        $1 != "host_is_html_dda" {
            print NR ":" $0
            bad=1
        }
    END { exit bad ? 0 : 1 }
' "$exclusions" >&2; then
    echo "error: for-await-of exclusion reason drifted" >&2
    exit 1
fi

awk -F'\t' '
    NR == FNR { selected[$1]=1; next }
    function has(list, value) {
        return index("," list ",", "," value ",") != 0
    }
    $1 in selected {
        reason=""
        if (has($4, "explicit-resource-management")) {
            reason="baseline_unsupported_erm"
        } else if (has($4, "dynamic-import") || has($4, "import.meta") ||
                   has($4, "top-level-await") || has($3, "module")) {
            reason="module_or_dynamic_import"
        } else if (has($4, "IsHTMLDDA")) {
            reason="host_is_html_dda"
        }
        if (reason != "") print reason "\t" $1
    }
' "$candidate" "$metadata_tsv" | LC_ALL=C sort -k2,2 > "$derived_exclusions"
diff -u "$exclusions" "$derived_exclusions"
awk -F'\t' '{ print $2 }' "$exclusions" > "$excluded_paths"
LC_ALL=C sort -c "$excluded_paths"
if [[ "$(inventory_count "$excluded_paths")" != "$(read_value excluded_paths)" \
    || "$(LC_ALL=C sort -u "$excluded_paths" | inventory_count /dev/stdin)" \
        != "$(read_value excluded_paths)" \
    || "$(sha256_file "$excluded_paths")" != "$(read_value excluded_paths_sha256)" ]]; then
    echo "error: for-await-of excluded path inventory drifted" >&2
    exit 1
fi
derive_variant_keys "$excluded_paths" "$excluded_keys"
if [[ "$(inventory_count "$excluded_keys")" != "$(read_value excluded_variants)" \
    || "$(sha256_file "$excluded_keys")" != "$(read_value excluded_keys_sha256)" ]]; then
    echo "error: for-await-of excluded variants drifted" >&2
    exit 1
fi
for reason in \
    baseline_unsupported_erm module_or_dynamic_import host_is_html_dda; do
    reason_inventory=$tmp_dir/excluded-$reason.txt
    awk -F'\t' -v reason="$reason" '$1 == reason { print $2 }' \
        "$exclusions" > "$reason_inventory"
    verify_inventory "excluded_$reason" "$reason_inventory"
done

comm -23 "$candidate" "$excluded_paths" > "$manifest"
LC_ALL=C sort -u "$manifest" "$excluded_paths" > "$partition_union"
diff -u "$candidate" "$partition_union"
if [[ -n "$(comm -12 "$manifest" "$excluded_paths")" ]]; then
    echo "error: for-await-of manifest and exclusions overlap" >&2
    exit 1
fi
if [[ "$(inventory_count "$manifest")" != "$(read_value paths)" \
    || "$(sha256_file "$manifest")" != "$(read_value manifest_sha256)" ]]; then
    echo "error: for-await-of admitted manifest drifted" >&2
    exit 1
fi

awk -F'\t' '
    function has(list, value) {
        return index("," list ",", "," value ",") != 0
    }
    $1 == "test/language/expressions/optional-chaining/iteration-statement-for-await-of.js" &&
        has($4, "optional-chaining") {
            print $1
        }
' "$metadata_tsv" > "$promoted_optional_chaining"
verify_inventory promoted_optional_chaining "$promoted_optional_chaining"
verify_variant_count "$promoted_optional_chaining" \
    "$(read_value promoted_optional_chaining_variants)" \
    "$promoted_optional_chaining_keys"
if [[ "$(sha256_file "$promoted_optional_chaining_keys")" \
        != "$(read_value promoted_optional_chaining_keys_sha256)" \
    || -n "$(comm -23 "$promoted_optional_chaining" "$manifest")" ]]; then
    echo "error: promoted optional-chaining for-await-of path drifted" >&2
    exit 1
fi

awk '/^test\/language\/statements\/for-await-of\// { print }' \
    "$manifest" > "$manifest_main"
awk '!/^test\/language\/statements\/for-await-of\// { print }' \
    "$manifest" > "$manifest_external"
verify_inventory main_directory "$manifest_main"
verify_variant_count "$manifest_main" "$(read_value main_directory_variants)" \
    "$tmp_dir/main-keys.txt"
verify_inventory external "$manifest_external"
verify_variant_count "$manifest_external" "$(read_value external_variants)" \
    "$tmp_dir/external-keys.txt"

awk '
    {
        name=$0
        sub(/^.*\//, "", name)
        if (name ~ /^async-(func|gen)(-decl)?-dstr-/) print
    }
' "$manifest_main" > "$destructuring"
comm -23 "$manifest_main" "$destructuring" > "$core_main"
if [[ "$(inventory_count "$destructuring")" != "$(read_value destructuring_paths)" \
    || "$(inventory_count "$core_main")" != "$(read_value core_main_directory_paths)" ]]; then
    echo "error: for-await-of main dependency partition drifted" >&2
    exit 1
fi
verify_variant_count "$destructuring" "$(read_value destructuring_variants)" \
    "$tmp_dir/destructuring-keys.txt"
verify_variant_count "$core_main" "$(read_value core_main_directory_variants)" \
    "$tmp_dir/core-main-keys.txt"

awk '/^test\/built-ins\/AsyncFromSyncIteratorPrototype\// { print }' \
    "$manifest" > "$async_from_sync"
awk '$0 == "test/language/expressions/await/for-await-of-interleaved.js" { print }' \
    "$manifest" > "$async_interleaving"
awk '/^test\/staging\/sm\/AsyncGenerators\/for-await-/ { print }' \
    "$manifest" > "$staging_grammar"
verify_inventory async_from_sync_iterator "$async_from_sync"
verify_variant_count "$async_from_sync" \
    "$(read_value async_from_sync_iterator_variants)" \
    "$tmp_dir/async-from-sync-keys.txt"
if [[ "$(inventory_count "$async_interleaving")" != "$(read_value async_interleaving_paths)" \
    || "$(inventory_count "$staging_grammar")" != "$(read_value staging_grammar_paths)" ]]; then
    echo "error: for-await-of external dependency partition drifted" >&2
    exit 1
fi
verify_variant_count "$async_interleaving" \
    "$(read_value async_interleaving_variants)" \
    "$tmp_dir/async-interleaving-keys.txt"
verify_variant_count "$staging_grammar" \
    "$(read_value staging_grammar_variants)" \
    "$tmp_dir/staging-grammar-keys.txt"

for index in 0 1 2 3; do
    ledger=${ledgers[$index]}
    ledger_hash_key=${ledger_hash_keys[$index]}
    shape_name=${shape_names[$index]}
    shape_inventory=$tmp_dir/$shape_name.txt
    if [[ "$(sha256_file "$ledger")" != "$(read_value "${ledger_hash_key}_sha256")" ]]; then
        echo "error: source async-generator exclusion ledger drifted: $ledger" >&2
        exit 1
    fi
    if awk -F'\t' \
        'NF != 2 || $1 == "" || $2 == "" { bad=1 } END { exit bad ? 0 : 1 }' \
        "$ledger"; then
        echo "error: malformed source async-generator exclusion ledger: $ledger" >&2
        exit 1
    fi
    awk -F'\t' '$1 == "for_await" { print $2 }' "$ledger" > "$shape_inventory"
    if [[ "$(inventory_count "$shape_inventory")" != "${ledger_for_await_counts[$index]}" ]]; then
        echo "error: source for_await partition drifted: $ledger" >&2
        exit 1
    fi
    verify_inventory "$shape_name" "$shape_inventory"
    cat "$shape_inventory" >> "$inherited_raw"
done
LC_ALL=C sort -u "$inherited_raw" > "$inherited"
verify_inventory inherited_for_await "$inherited"
verify_variant_count "$inherited" "$(read_value inherited_for_await_variants)" \
    "$tmp_dir/inherited-keys.txt"
if [[ -n "$(comm -23 "$inherited" "$manifest")" ]]; then
    echo "error: inherited for_await ledger path escaped the R3bk manifest" >&2
    exit 1
fi

printf '%s\n' \
    Symbol \
    Symbol.asyncIterator \
    Symbol.iterator \
    async-functions \
    async-iteration \
    class \
    class-methods-private \
    class-static-methods-private \
    const \
    destructuring-binding \
    generators \
    object-rest \
    optional-chaining > "$expected_features"
printf '%s\n' \
    asyncHelpers.js \
    compareArray.js \
    propertyHelper.js > "$expected_includes"
printf '%s\n' async generated noStrict onlyStrict > "$expected_flags"
profile_section audited-negative-tests | LC_ALL=C sort > "$expected_negative"
diff -u "$expected_features" <(profile_section features | LC_ALL=C sort)
[[ "$(profile_section execution)" == "async=true" ]] \
    || { echo "error: for-await-of profile must opt into only the async host" >&2; exit 1; }

awk -F'\t' \
    -v expected="$(read_value paths)" \
    -v positive="$positive" \
    -v negative="$negative" \
    -v async_paths="$async_paths" \
    -v sync_paths="$sync_paths" \
    -v double_mode="$double_mode" \
    -v no_strict="$no_strict" \
    -v only_strict="$only_strict" \
    -v variant_keys="$variant_keys" \
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
            print "unsupported admitted mode metadata: " $1 > "/dev/stderr"
            exit 2
        } else if (has($3, "noStrict")) {
            print $1 > no_strict
            print $1 "\tsloppy" > variant_keys
        } else if (has($3, "onlyStrict")) {
            print $1 > only_strict
            print $1 "\tstrict" > variant_keys
        } else {
            print $1 > double_mode
            print $1 "\tsloppy" > variant_keys
            print $1 "\tstrict" > variant_keys
        }
        count=split($4, values, ",")
        for (i=1; i<=count; i++) if (values[i] != "") print values[i] > feature_occurrences
        count=split($2, values, ",")
        for (i=1; i<=count; i++) if (values[i] != "") print values[i] > include_occurrences
        count=split($3, values, ",")
        for (i=1; i<=count; i++) if (values[i] != "") print values[i] > flag_occurrences
    }
    END {
        if (seen != expected) {
            print "selected metadata rows: " seen > "/dev/stderr"
            exit 2
        }
    }
' "$manifest" "$metadata_tsv"

for inventory in \
    "$positive" "$negative" "$async_paths" "$sync_paths" "$double_mode" \
    "$no_strict" "$only_strict" "$variant_keys"; do
    LC_ALL=C sort "$inventory" -o "$inventory"
done
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
    || "$(sha256_file "$variant_keys")" != "$(read_value keys_sha256)" ]]; then
    echo "error: for-await-of admitted variant keys drifted" >&2
    exit 1
fi
verify_plain_inventory features "$feature_inventory"
verify_plain_inventory includes "$include_inventory"
verify_plain_inventory flags "$flag_inventory"
diff -u "$expected_features" "$feature_inventory"
diff -u "$expected_includes" "$include_inventory"
diff -u "$expected_flags" "$flag_inventory"
diff -u "$expected_negative" "$negative"

verify_quickjs_oracle \
    "$candidate" "$(read_value quickjs_candidate_enabled_passes)" \
    "$candidate_quickjs_log" "complete baseline-enabled for-await-of candidate"
verify_quickjs_oracle \
    "$manifest" "$(read_value quickjs_passes)" \
    "$quickjs_log" "R3bk for-await-of"
if "$check_only"; then
    printf 'for-await-of inputs verified: %s candidates - %s exclusions = %s paths / %s variants; pinned QuickJS passes %s/%s admitted paths\n' \
        "$(read_value candidate_paths)" "$(read_value excluded_paths)" \
        "$(read_value paths)" "$(read_value variants)" \
        "$(read_value quickjs_passes)" "$(read_value paths)"
    exit 0
fi

repeat_report=target/test262-for-await-of-repeat.tsv
five_report=target/test262-for-await-of-workers-5.tsv
rm -f -- "$report" "$json_report" \
    "$repeat_report" "${repeat_report%.tsv}.jsonl" \
    "$five_report" "${five_report%.tsv}.jsonl"
run_output=$(run_oxide "$workers" "$report")
printf '%s\n' "$run_output"

actual_variants=$(awk -F'\t' '
    !/^#/ && !($1 == "path" && $2 == "variant") { count++ }
    END { print count + 0 }
' "$report")
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
actual_runnable=$(printf '%s\n' "$run_output" | awk '
    /^execution: runnable=/ {
        sub(/^execution: runnable=/, "")
        sub(/ .*/, "")
        print
        found=1
    }
    END { if (!found) exit 1 }
')
actual_nonpass=$(awk -F'\t' '
    !/^#/ && !($1 == "path" && $2 == "variant") && $7 != "pass" {
        print $1 "\t" $2 "\t" $7 "\t" $8 "\t" $9 "\t" $10
    }
' "$report" | sha256_stream)
actual_summary=$(tail -n 1 "$report" | sed 's/^# summary //')

if [[ "$(read_header quickjs)" != "$(read_value quickjs)" \
    || "$(read_header test262)" != "$(read_value test262)" \
    || "$(read_header test262_patch_sha256)" != "$(read_value test262_patch_sha256)" \
    || "$(read_header test262_config_sha256)" != "$(read_value test262_config_sha256)" \
    || "$(read_header test262_metadata_sha256)" != "$(read_value test262_metadata_sha256)" \
    || "$(read_header oxide_profile_sha256)" != "$(read_value oxide_profile_sha256)" \
    || "$(read_header profile)" != "$(read_value schema)" \
    || "$(read_header mode)" != "$(read_value mode)" \
    || "$actual_variants" != "$(read_value variants)" \
    || "$actual_runnable" != "$(read_value variants)" ]]; then
    echo "error: for-await-of report metadata drifted" >&2
    exit 1
fi
diff -u "$variant_keys" <(awk -F'\t' '
    !/^#/ && !($1 == "path" && $2 == "variant") { print $1 "\t" $2 }
' "$report" | LC_ALL=C sort)

if [[ "$actual_passes" != "$(read_value variants)" \
    || "$actual_failures" != 0 \
    || "$actual_unsupported" != 0 \
    || "$actual_skipped" != 0 ]]; then
    echo "error: for-await-of Test262 cohort is not all-pass" >&2
    awk -F'\t' '
        !/^#/ && !($1 == "path" && $2 == "variant") && $7 != "pass" {
            print $1 "\t" $2 "\t" $7 "\t" $8 "\t" $9 "\t" $10
        }
    ' "$report" >&2
    exit 1
fi

run_oxide "$workers" "$repeat_report" >/dev/null
run_oxide 5 "$five_report" >/dev/null
cmp -s "$report" "$repeat_report"
cmp -s "${report%.tsv}.jsonl" "${repeat_report%.tsv}.jsonl"
cmp -s "$report" "$five_report"
cmp -s "${report%.tsv}.jsonl" "${five_report%.tsv}.jsonl"

pending=false
for key in runnable passes failures unsupported skipped nonpass_sha256 tsv_sha256 jsonl_sha256 summary; do
    if [[ "$(read_value "$key")" == "PENDING" ]]; then
        pending=true
    fi
done
if "$pending"; then
    echo "error: Oxide passed, but the R3bk baseline is intentionally still PENDING" >&2
    printf 'runnable=%s\npasses=%s\nfailures=%s\nunsupported=%s\nskipped=%s\n' \
        "$actual_runnable" "$actual_passes" "$actual_failures" \
        "$actual_unsupported" "$actual_skipped" >&2
    printf 'nonpass_sha256=%s\ntsv_sha256=%s\njsonl_sha256=%s\nsummary=%s\n' \
        "$actual_nonpass" "$(sha256_file "$report")" \
        "$(sha256_file "$json_report")" "$actual_summary" >&2
    exit 1
fi

if [[ "$actual_runnable" != "$(read_value runnable)" \
    || "$actual_passes" != "$(read_value passes)" \
    || "$actual_failures" != "$(read_value failures)" \
    || "$actual_unsupported" != "$(read_value unsupported)" \
    || "$actual_skipped" != "$(read_value skipped)" \
    || "$actual_nonpass" != "$(read_value nonpass_sha256)" \
    || "$(sha256_file "$report")" != "$(read_value tsv_sha256)" \
    || "$(sha256_file "$json_report")" != "$(read_value jsonl_sha256)" \
    || "$actual_summary" != "$(read_value summary)" ]]; then
    echo "error: for-await-of all-pass baseline drifted" >&2
    exit 1
fi

printf 'for-await-of Test262 gate passes: %s/%s variants across %s paths; %s/%s/5-worker reports are byte-identical\n' \
    "$actual_passes" "$actual_variants" "$(read_value paths)" \
    "$workers" "$workers"
