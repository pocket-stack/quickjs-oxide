#!/usr/bin/env bash
# Reproduce the R3aj async-generator yield-star Test262 gate.

set -euo pipefail
export TZ=America/Los_Angeles

script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
root=$(CDPATH= cd -- "$script_dir/.." && pwd)
baseline=tests/test262-async-generator-yield-star-baseline.txt
manifest=tests/test262-async-generator-yield-star.txt
admission_profile=tests/test262-async-generator-yield-star.conf
global_profile=compat/test262-oxide.conf
report=target/test262-async-generator-yield-star.tsv
quickjs_log=target/test262-async-generator-yield-star-quickjs.log
ledgers=(
    tests/test262-async-generator-core-exclusions.tsv
    tests/test262-async-generator-object-method-core-exclusions.tsv
    tests/test262-async-generator-class-method-core-exclusions.tsv
    tests/test262-async-generator-private-class-method-core-exclusions.tsv
)
ledger_names=(
    async_generator_core
    async_generator_object_method_core
    async_generator_class_method_core
    async_generator_private_class_method_core
)
ledger_yield_star_counts=(185 58 232 300)

usage() {
    printf 'usage: %s [--check]\n' "${0##*/}"
    printf '  --check  verify frozen inputs, metadata, and pinned QuickJS; skip Oxide\n'
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
        echo "error: async-generator yield-star baseline $key drifted" >&2
        exit 1
    fi
}

verify_inventory() {
    local name=$1 inventory=$2
    if [[ "$(inventory_count "$inventory")" != "$(read_value "${name}")" \
        || "$(sha256_file "$inventory")" != "$(read_value "${name}_sha256")" ]]; then
        echo "error: async-generator yield-star $name inventory drifted" >&2
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

    if ! (cd -- "$source_dir" \
        && ./run-test262 -a -m -c test262.conf -f "${files[@]}") \
        >"$quickjs_log" 2>&1; then
        cat "$quickjs_log" >&2
        echo "error: pinned QuickJS could not execute the yield-star cohort" >&2
        exit 1
    fi
    if grep -Eq '(^|[[:space:]])FAILED($|[[:space:]])' "$quickjs_log" \
        || ! grep -Fq \
            "Average memory statistics for $(read_value quickjs_passes) tests:" \
            "$quickjs_log"; then
        cat "$quickjs_log" >&2
        echo "error: pinned QuickJS no longer passes the yield-star cohort" >&2
        exit 1
    fi
}

run_oxide() {
    local workers=$1 output_report=$2
    cargo run --locked --release --quiet --bin run-test262 -- \
        --suite "$suite" \
        --config "$source_dir/test262.conf" \
        --oxide-profile "$admission_profile" \
        --manifest "$manifest" \
        --report "$output_report" \
        --mode "$(read_value mode)" \
        --workers "$workers" \
        --timeout-ms "$(read_value timeout_ms)" \
        --allow-failures
}

cd -- "$root"
suite=$("$script_dir/prepare-test262.sh")
source_dir=$(dirname -- "$suite")
tmp_dir=$(mktemp -d "${TMPDIR:-/tmp}/quickjs-oxide-r3aj.XXXXXX")
trap 'rm -rf -- "$tmp_dir"' EXIT HUP INT TERM

derived_raw=$tmp_dir/derived-raw.txt
derived_manifest=$tmp_dir/manifest.txt
tracked_tests=$tmp_dir/tracked-tests.txt
missing_tests=$tmp_dir/missing-tests.txt
metadata_records=$tmp_dir/metadata.records
metadata_tsv=$tmp_dir/metadata.tsv
positive=$tmp_dir/positive.txt
negative=$tmp_dir/negative.txt
async_paths=$tmp_dir/async.txt
sync_paths=$tmp_dir/sync.txt
variant_keys=$tmp_dir/variant-keys.txt
feature_occurrences=$tmp_dir/features.raw
include_occurrences=$tmp_dir/includes.raw
flag_occurrences=$tmp_dir/flags.raw
feature_inventory=$tmp_dir/features.txt
include_inventory=$tmp_dir/includes.txt
flag_inventory=$tmp_dir/flags.txt
expected_features=$tmp_dir/expected-features.txt
expected_negative=$tmp_dir/expected-negative.txt
expected_flags=$tmp_dir/expected-flags.txt

: > "$derived_raw"
: > "$positive"
: > "$negative"
: > "$async_paths"
: > "$sync_paths"
: > "$variant_keys"
: > "$feature_occurrences"
: > "$include_occurrences"
: > "$flag_occurrences"

expect_value quickjs 2026-06-04
expect_value test262 5c8206929d81b2d3d727ca6aac56c18358c8d790
expect_value test262_patch_sha256 f4b23b04641d438df0826fb17d7a5db276af2bdb085b42cc09aa8d50e0da9ba3
expect_value test262_config_sha256 79c64748ff1182baf5433d0a8378e3666738a785d02faf71f0d459ed42ae897b
expect_value test262_metadata_sha256 a37219960819e56a5c5c1723d31d6a33095c778bf5347385187fde96f927a06a
expect_value global_oxide_profile_sha256 fc6e8010c982bd6324b146e5f8e3ea0592aac7c03a323a8dbc8d778b4b670b23
expect_value schema test262-canonical-classified-v2
expect_value mode both
expect_value timeout_ms 30000
expect_value paths 775
expect_value quickjs_passes 775
expect_value positive_paths 774
expect_value negative_paths 1
expect_value async_paths 774
expect_value sync_paths 1
expect_value variants 1550
expect_value features 9
expect_value includes 2
expect_value flags 2

if [[ "$(sha256_file "$global_profile")" != "$(read_value global_oxide_profile_sha256)" \
    || "$(sha256_file "$admission_profile")" != "$(read_value oxide_profile_sha256)" ]]; then
    echo "error: async-generator yield-star capability profile drifted" >&2
    exit 1
fi
if [[ "$(profile_section execution "$global_profile")" != "async=true" ]]; then
    echo "error: global Test262 profile must admit only async execution" >&2
    exit 1
fi

for index in 0 1 2 3; do
    ledger=${ledgers[$index]}
    name=${ledger_names[$index]}
    if [[ "$(sha256_file "$ledger")" != "$(read_value "${name}_exclusions_sha256")" ]]; then
        echo "error: source exclusion ledger drifted: $ledger" >&2
        exit 1
    fi
    if awk -F'\t' \
        'NF != 2 || $1 == "" || $2 == "" { print NR ":" $0; bad=1 } END { exit bad ? 0 : 1 }' \
        "$ledger" >&2; then
        echo "error: malformed source exclusion ledger: $ledger" >&2
        exit 1
    fi
    actual_count=$(awk -F'\t' '$1 == "yield_star" { count++ } END { print count + 0 }' "$ledger")
    if [[ "$actual_count" != "${ledger_yield_star_counts[$index]}" ]]; then
        echo "error: yield_star partition drifted in $ledger" >&2
        exit 1
    fi
    awk -F'\t' '$1 == "yield_star" { print $2 }' "$ledger" >> "$derived_raw"
done

LC_ALL=C sort "$derived_raw" -o "$derived_raw"
LC_ALL=C sort -u "$derived_raw" > "$derived_manifest"
LC_ALL=C sort -c "$manifest"
diff -u "$manifest" "$derived_manifest"
if [[ "$(inventory_count "$derived_raw")" != "$(read_value paths)" \
    || "$(inventory_count "$derived_manifest")" != "$(read_value paths)" \
    || "$(sha256_file "$manifest")" != "$(read_value manifest_sha256)" ]]; then
    echo "error: async-generator yield-star manifest derivation drifted" >&2
    exit 1
fi

git -C "$suite" ls-files test | LC_ALL=C sort > "$tracked_tests"
comm -23 "$manifest" "$tracked_tests" > "$missing_tests"
if [[ -s "$missing_tests" ]]; then
    echo "error: yield-star manifest contains paths outside pinned Test262" >&2
    cat "$missing_tests" >&2
    exit 1
fi

printf '%s\n' \
    Symbol.asyncIterator \
    Symbol.iterator \
    async-functions \
    async-iteration \
    class \
    class-fields-public \
    class-methods-private \
    class-static-methods-private \
    generators > "$expected_features"
printf '%s\n' \
    test/language/expressions/async-generator/early-errors-expression-yield-star-after-newline.js \
    > "$expected_negative"
printf '%s\n' async generated > "$expected_flags"
diff -u "$expected_features" <(profile_section features | LC_ALL=C sort)
diff -u "$expected_negative" <(profile_section audited-negative-tests | LC_ALL=C sort)
[[ "$(profile_section execution)" == "async=true" ]] \
    || { echo "error: yield-star profile must opt into only the async host" >&2; exit 1; }

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
            if ($1 != "test/language/expressions/async-generator/early-errors-expression-yield-star-after-newline.js" \
                || $5 != "parse" || $6 != "SyntaxError") {
                print "bad negative provenance: " $1 > "/dev/stderr"
                exit 2
            }
            print $1 > negative
        }
        if (has($3, "async")) print $1 > async_paths
        else print $1 > sync_paths
        if (has($3, "module") || has($3, "raw") \
            || has($3, "noStrict") || has($3, "onlyStrict")) {
            print "unsupported mode metadata: " $1 > "/dev/stderr"
            exit 2
        }
        print $1 "\tsloppy" > variant_keys
        print $1 "\tstrict" > variant_keys
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

LC_ALL=C sort "$positive" -o "$positive"
LC_ALL=C sort "$negative" -o "$negative"
LC_ALL=C sort "$async_paths" -o "$async_paths"
LC_ALL=C sort "$sync_paths" -o "$sync_paths"
LC_ALL=C sort "$variant_keys" -o "$variant_keys"
LC_ALL=C sort -u "$feature_occurrences" > "$feature_inventory"
LC_ALL=C sort -u "$include_occurrences" > "$include_inventory"
LC_ALL=C sort -u "$flag_occurrences" > "$flag_inventory"

if [[ "$(inventory_count "$positive")" != "$(read_value positive_paths)" \
    || "$(inventory_count "$negative")" != "$(read_value negative_paths)" \
    || "$(inventory_count "$async_paths")" != "$(read_value async_paths)" \
    || "$(inventory_count "$sync_paths")" != "$(read_value sync_paths)" \
    || "$(inventory_count "$variant_keys")" != "$(read_value variants)" \
    || "$(sha256_file "$variant_keys")" != "$(read_value keys_sha256)" ]]; then
    echo "error: async-generator yield-star metadata partition drifted" >&2
    exit 1
fi
verify_inventory features "$feature_inventory"
verify_inventory includes "$include_inventory"
verify_inventory flags "$flag_inventory"
diff -u "$expected_features" "$feature_inventory"
diff -u "$expected_negative" "$negative"
diff -u "$expected_flags" "$flag_inventory"

verify_quickjs_oracle
if "$check_only"; then
    printf 'async-generator yield-star inputs verified: %s paths, %s variants; pinned QuickJS passes %s/%s\n' \
        "$(read_value paths)" "$(read_value variants)" \
        "$(read_value quickjs_passes)" "$(read_value paths)"
    exit 0
fi

repeat_report=target/test262-async-generator-yield-star-repeat.tsv
five_report=target/test262-async-generator-yield-star-workers-5.tsv
rm -f -- "$report" "${report%.tsv}.jsonl" \
    "$repeat_report" "${repeat_report%.tsv}.jsonl" \
    "$five_report" "${five_report%.tsv}.jsonl"
run_output=$(run_oxide 8 "$report")
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
    echo "error: async-generator yield-star report metadata drifted" >&2
    exit 1
fi
diff -u "$variant_keys" <(awk -F'\t' '
    !/^#/ && !($1 == "path" && $2 == "variant") { print $1 "\t" $2 }
' "$report" | LC_ALL=C sort)

if [[ "$actual_passes" != "$(read_value variants)" \
    || "$actual_failures" != 0 \
    || "$actual_unsupported" != 0 \
    || "$actual_skipped" != 0 ]]; then
    echo "error: async-generator yield-star Test262 cohort is not all-pass" >&2
    awk -F'\t' '
        !/^#/ && !($1 == "path" && $2 == "variant") && $7 != "pass" {
            print $1 "\t" $2 "\t" $7 "\t" $8 "\t" $9 "\t" $10
        }
    ' "$report" >&2
    exit 1
fi

run_oxide 8 "$repeat_report" >/dev/null
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
    echo "error: Oxide passed, but the R3aj baseline is intentionally still PENDING" >&2
    printf 'runnable=%s\npasses=%s\nfailures=%s\nunsupported=%s\nskipped=%s\n' \
        "$actual_runnable" "$actual_passes" "$actual_failures" \
        "$actual_unsupported" "$actual_skipped" >&2
    printf 'nonpass_sha256=%s\ntsv_sha256=%s\njsonl_sha256=%s\nsummary=%s\n' \
        "$actual_nonpass" "$(sha256_file "$report")" \
        "$(sha256_file "${report%.tsv}.jsonl")" "$actual_summary" >&2
    exit 1
fi

if [[ "$actual_runnable" != "$(read_value runnable)" \
    || "$actual_passes" != "$(read_value passes)" \
    || "$actual_failures" != "$(read_value failures)" \
    || "$actual_unsupported" != "$(read_value unsupported)" \
    || "$actual_skipped" != "$(read_value skipped)" \
    || "$actual_nonpass" != "$(read_value nonpass_sha256)" \
    || "$(sha256_file "$report")" != "$(read_value tsv_sha256)" \
    || "$(sha256_file "${report%.tsv}.jsonl")" != "$(read_value jsonl_sha256)" \
    || "$actual_summary" != "$(read_value summary)" ]]; then
    echo "error: async-generator yield-star all-pass baseline drifted" >&2
    exit 1
fi

printf 'async-generator yield-star Test262 gate passes: %s/%s variants across %s paths; 8/8/5-worker reports are byte-identical\n' \
    "$actual_passes" "$actual_variants" "$(read_value paths)"
