#!/usr/bin/env bash
# Reproduce the R3al global async Test262 admission gate.

set -euo pipefail
export TZ=America/Los_Angeles

script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
root=$(CDPATH= cd -- "$script_dir/.." && pwd)
baseline=tests/test262-global-async-baseline.txt
manifest=tests/test262-global-async.txt
r3ak_before=tests/test262-global-async-r3ak-before.tsv
profile=compat/test262-oxide.conf
report=target/test262-global-async.tsv
json_report=target/test262-global-async.jsonl
quickjs_log=target/test262-global-async-quickjs.log

readonly R3AK_FULL_TSV_SHA256=36e2a11f4eaba4ffd92fdd561b18b27337b90b14a564cab9da6385f1aa0f79a3
readonly R3AK_PROFILE_SHA256=6a4d3dc37da05f6e63d7b8564483159c383ed66c665a2b5530624e628f73b908
readonly R3AK_BEFORE_SHA256=173d61580131172206cb476a4239395a5a258d539723587d924d161eb12d461f
readonly R3AK_BEFORE_KEYS_SHA256=6d888787cb21790babb173d93d3a73df58ebaf323b87dcc8ec35cb4041e84bfc
readonly R3AK_R3AL_TRANSITIONS_SHA256=eae7dd348199be707bdd914e1d8be2eb5bf63a17ee7c93ef96548e915e57b1d8

usage() {
    printf 'usage: %s [--check]\n' "${0##*/}"
    printf '  --check  verify frozen R3ak/metadata/profile inputs and pinned QuickJS; skip Oxide\n'
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
    local input=$1 key=$2
    awk -F= -v key="# $key" '
        $1 == key { sub(/^[^=]*=/, ""); print; found=1 }
        END { if (!found) exit 1 }
    ' "$input"
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
    local section=$1
    awk -v section="[$section]" '
        $0 == section { inside=1; next }
        /^\[/ { inside=0 }
        inside && NF && $1 !~ /^#/ { print }
    ' "$profile"
}

verify_r3ak_before() {
    local actual_candidate_paths actual_candidate_variants
    local actual_unsupported_async actual_unsupported_feature
    local actual_unsupported_module_async

    if [[ "$(sha256_file "$r3ak_before")" != "$R3AK_BEFORE_SHA256" \
        || "$(read_header "$r3ak_before" source_tsv_sha256)" != "$R3AK_FULL_TSV_SHA256" \
        || "$(read_header "$r3ak_before" oxide_profile_sha256)" != "$R3AK_PROFILE_SHA256" ]]; then
        echo "error: frozen R3ak before-outcome provenance drifted" >&2
        exit 1
    fi

    if ! awk -F'\t' '
        /^#/ { next }
        $1 == "path" {
            if ($0 != "path\tvariant\tbefore_outcome\tmissing_requirement") exit 1
            header++
            next
        }
        {
            if (NF != 4 || ($2 != "sloppy" && $2 != "strict")) exit 1
            if ($3 == "unsupported-async") {
                if ($4 !~ /(^|, )async(, |$)/) exit 1
                unsupported_async++
            } else if ($3 == "unsupported-module-async") {
                if ($4 !~ /(^|, )module(, |$)/ ||
                    $4 !~ /(^|, )async(, |$)/) exit 1
                unsupported_module_async++
            } else if ($3 == "unsupported-feature") {
                if ($4 !~ /(^|, )(async-functions|async-iteration)(, |$)/) exit 1
                unsupported_feature++
            } else {
                exit 1
            }
            rows++
        }
        END {
            if (header != 1 || rows == 0 ||
                unsupported_async == 0 ||
                unsupported_module_async == 0 ||
                unsupported_feature == 0) exit 1
        }
    ' "$r3ak_before"; then
        echo "error: malformed R3ak before-outcome provenance" >&2
        exit 1
    fi

    awk -F'\t' '!/^#/ && $1 != "path" { print $1 "\t" $2 }' \
        "$r3ak_before" > "$r3ak_before_keys"
    if ! LC_ALL=C sort -c "$r3ak_before_keys" \
        || [[ "$(LC_ALL=C sort -u "$r3ak_before_keys" | inventory_count /dev/stdin)" \
            != "$(inventory_count "$r3ak_before_keys")" ]] \
        || [[ "$(sha256_file "$r3ak_before_keys")" != "$R3AK_BEFORE_KEYS_SHA256" ]]; then
        echo "error: R3ak before-outcome keys are not canonical" >&2
        exit 1
    fi

    cut -f1 "$r3ak_before_keys" | LC_ALL=C sort -u > "$r3ak_candidate_paths"
    actual_candidate_paths=$(inventory_count "$r3ak_candidate_paths")
    actual_candidate_variants=$(inventory_count "$r3ak_before_keys")
    actual_unsupported_async=$(awk -F'\t' '$3 == "unsupported-async" { count++ }
        END { print count + 0 }' "$r3ak_before")
    actual_unsupported_feature=$(awk -F'\t' '$3 == "unsupported-feature" { count++ }
        END { print count + 0 }' "$r3ak_before")
    actual_unsupported_module_async=$(awk -F'\t' \
        '$3 == "unsupported-module-async" { count++ }
        END { print count + 0 }' "$r3ak_before")

    if [[ "$actual_candidate_paths" != "$(read_value r3ak_candidate_paths)" \
        || "$actual_candidate_variants" != "$(read_value r3ak_candidate_variants)" \
        || "$actual_unsupported_async" != "$(read_value r3ak_unsupported_async)" \
        || "$actual_unsupported_feature" != "$(read_value r3ak_unsupported_feature)" \
        || "$actual_unsupported_module_async" \
            != "$(read_value r3ak_unsupported_module_async)" ]]; then
        echo "error: R3ak before-outcome candidate inventory drifted" >&2
        exit 1
    fi
}

expect_value() {
    local key=$1 expected=$2
    if [[ "$(read_value "$key")" != "$expected" ]]; then
        echo "error: global async baseline $key drifted" >&2
        exit 1
    fi
}

verify_count() {
    local key=$1 inventory=$2
    if [[ "$(inventory_count "$inventory")" != "$(read_value "$key")" ]]; then
        echo "error: global async $key inventory drifted" >&2
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
        tail -n 200 "$quickjs_log" >&2
        echo "error: pinned QuickJS could not execute the global async cohort" >&2
        exit 1
    fi
    if grep -Eq '(^|[[:space:]])FAILED($|[[:space:]])' "$quickjs_log" \
        || ! grep -Fq \
            "Average memory statistics for $(read_value quickjs_passes) tests:" \
            "$quickjs_log"; then
        tail -n 200 "$quickjs_log" >&2
        echo "error: pinned QuickJS no longer passes the global async cohort" >&2
        exit 1
    fi
}

run_oxide() {
    local worker_count=$1 output_report=$2
    cargo run --locked --release --quiet --bin run-test262 -- \
        --suite "$suite" \
        --config "$source_dir/test262.conf" \
        --oxide-profile "$profile" \
        --manifest "$manifest" \
        --report "$output_report" \
        --mode "$(read_value mode)" \
        --workers "$worker_count" \
        --timeout-ms "$(read_value timeout_ms)"
}

run_r3al_candidates() {
    local worker_count=$1 output_report=$2
    cargo run --locked --release --quiet --bin run-test262 -- \
        --suite "$suite" \
        --config "$source_dir/test262.conf" \
        --oxide-profile "$profile" \
        --manifest "$r3ak_candidate_paths" \
        --report "$output_report" \
        --mode "$(read_value mode)" \
        --workers "$worker_count" \
        --timeout-ms "$(read_value timeout_ms)" \
        --allow-failures
}

verify_transition_join() {
    local input_report=$1 run_output=$2
    local actual_variants execution_line actual_runnable actual_unsupported
    local actual_summary

    actual_variants=$(awk -F'\t' '
        !/^#/ && !($1 == "path" && $2 == "variant") { count++ }
        END { print count + 0 }
    ' "$input_report")
    execution_line=$(printf '%s\n' "$run_output" \
        | awk '/^execution: runnable=/ { print; found=1 } END { if (!found) exit 1 }')
    actual_runnable=${execution_line#*runnable=}
    actual_runnable=${actual_runnable%% *}
    actual_unsupported=$(awk -F'\t' '
        !/^#/ && !($1 == "path" && $2 == "variant") && $7 ~ /^unsupported-/ {
            count++
        }
        END { print count + 0 }
    ' "$input_report")
    actual_summary=$(tail -n 1 "$input_report" | sed 's/^# summary //')

    if [[ "$(read_header "$input_report" quickjs)" != "$(read_value quickjs)" \
        || "$(read_header "$input_report" test262)" != "$(read_value test262)" \
        || "$(read_header "$input_report" test262_patch_sha256)" \
            != "$(read_value test262_patch_sha256)" \
        || "$(read_header "$input_report" test262_config_sha256)" \
            != "$(read_value test262_config_sha256)" \
        || "$(read_header "$input_report" test262_metadata_sha256)" \
            != "$(read_value test262_metadata_sha256)" \
        || "$(read_header "$input_report" oxide_profile_sha256)" \
            != "$(read_value oxide_profile_sha256)" \
        || "$(read_header "$input_report" profile)" != "$(read_value schema)" \
        || "$(read_header "$input_report" mode)" != "$(read_value mode)" \
        || "$actual_variants" != "$(read_value r3ak_candidate_variants)" \
        || "$actual_runnable" != "$(read_value newly_runnable)" \
        || "$actual_unsupported" != "$(read_value r3al_candidate_unsupported)" \
        || "$actual_summary" != "$(read_value r3al_candidate_summary)" ]]; then
        echo "error: R3al candidate report drifted from the frozen R3ak universe" >&2
        exit 1
    fi

    awk -F'\t' '
        !/^#/ && !($1 == "path" && $2 == "variant") { print $1 "\t" $2 }
    ' "$input_report" | LC_ALL=C sort > "$r3al_candidate_keys"
    if ! cmp -s "$r3ak_before_keys" "$r3al_candidate_keys"; then
        echo "error: R3ak/R3al candidate key universe drifted" >&2
        diff -u "$r3ak_before_keys" "$r3al_candidate_keys" >&2 || true
        exit 1
    fi

    awk -F'\t' '
        NR == FNR && !/^#/ && $1 != "path" {
            before[$1 FS $2]=$3
            next
        }
        !/^#/ && !($1 == "path" && $2 == "variant") {
            key=$1 FS $2
            if (!(key in before)) exit 1
            print $1 FS $2 FS before[key] FS $7
        }
    ' "$r3ak_before" "$input_report" > "$transition_vector"
    if [[ "$(sha256_file "$transition_vector")" \
        != "$R3AK_R3AL_TRANSITIONS_SHA256" ]]; then
        echo "error: exact R3ak/R3al transition vector drifted" >&2
        awk -F'\t' '{
            counts[$3 " -> " $4]++
        } END {
            for (transition in counts) print counts[transition], transition
        }' "$transition_vector" | LC_ALL=C sort >&2
        exit 1
    fi

    awk -F'\t' '
        $4 !~ /^unsupported-/ && $4 !~ /^skipped-/ {
            print $1 "\t" $2
        }
    ' "$transition_vector" > "$newly_runnable_keys"
    if [[ "$(inventory_count "$newly_runnable_keys")" != "$(read_value newly_runnable)" \
        || "$(sha256_file "$newly_runnable_keys")" \
            != "$(read_value keys_sha256)" ]]; then
        echo "error: R3ak/R3al newly-runnable key identity drifted" >&2
        exit 1
    fi
    if ! cmp -s "$variant_keys" "$newly_runnable_keys"; then
        echo "error: global async manifest is not the exhaustive R3ak/R3al transition" >&2
        echo "missing from manifest:" >&2
        comm -13 "$variant_keys" "$newly_runnable_keys" >&2
        echo "not newly runnable from R3ak:" >&2
        comm -23 "$variant_keys" "$newly_runnable_keys" >&2
        exit 1
    fi
}

verify_report() {
    local input_report=$1 input_json=$2 run_output=$3
    local actual_variants actual_runnable execution_line actual_keys
    local actual_passes actual_unsupported actual_skipped actual_failures
    local actual_nonpass actual_summary runner_summary expected_runner_summary

    actual_variants=$(awk -F'\t' '
        !/^#/ && !($1 == "path" && $2 == "variant") { count++ }
        END { print count + 0 }
    ' "$input_report")
    execution_line=$(printf '%s\n' "$run_output" \
        | awk '/^execution: runnable=/ { print; found=1 } END { if (!found) exit 1 }')
    actual_runnable=${execution_line#*runnable=}
    actual_runnable=${actual_runnable%% *}

    if [[ "$(read_header "$input_report" quickjs)" != "$(read_value quickjs)" \
        || "$(read_header "$input_report" test262)" != "$(read_value test262)" \
        || "$(read_header "$input_report" test262_patch_sha256)" != "$(read_value test262_patch_sha256)" \
        || "$(read_header "$input_report" test262_config_sha256)" != "$(read_value test262_config_sha256)" \
        || "$(read_header "$input_report" test262_metadata_sha256)" != "$(read_value test262_metadata_sha256)" \
        || "$(read_header "$input_report" oxide_profile_sha256)" != "$(read_value oxide_profile_sha256)" \
        || "$(read_header "$input_report" profile)" != "$(read_value schema)" \
        || "$(read_header "$input_report" mode)" != "$(read_value mode)" \
        || "$actual_variants" != "$(read_value variants)" \
        || "$actual_runnable" != "$(read_value runnable)" ]]; then
        echo "error: global async report metadata drifted" >&2
        exit 1
    fi

    actual_keys=$(awk -F'\t' '
        !/^#/ && !($1 == "path" && $2 == "variant") { print $1 "\t" $2 }
    ' "$input_report" | LC_ALL=C sort | sha256_stream)
    actual_passes=$(awk -F'\t' '
        !/^#/ && !($1 == "path" && $2 == "variant") && $7 == "pass" { count++ }
        END { print count + 0 }
    ' "$input_report")
    actual_unsupported=$(awk -F'\t' '
        !/^#/ && !($1 == "path" && $2 == "variant") && $7 ~ /^unsupported-/ { count++ }
        END { print count + 0 }
    ' "$input_report")
    actual_skipped=$(awk -F'\t' '
        !/^#/ && !($1 == "path" && $2 == "variant") && $7 ~ /^skipped-/ { count++ }
        END { print count + 0 }
    ' "$input_report")
    actual_failures=$((actual_variants - actual_passes - actual_unsupported - actual_skipped))
    actual_nonpass=$(awk -F'\t' '
        !/^#/ && !($1 == "path" && $2 == "variant") && $7 != "pass" {
            print $1 "\t" $2 "\t" $7 "\t" $8 "\t" $9 "\t" $10
        }
    ' "$input_report" | sha256_stream)
    actual_summary=$(tail -n 1 "$input_report" | sed 's/^# summary //')
    runner_summary=$(printf '%s\n' "$run_output" \
        | awk '/^Test262: total=/ { print; found=1 } END { if (!found) exit 1 }')
    expected_runner_summary="Test262: total=$(read_value variants) pass=$(read_value passes) fail=$(read_value failures) unsupported=$(read_value unsupported) skipped=$(read_value skipped)"

    if [[ "$runner_summary" != "$expected_runner_summary" \
        || "$actual_passes" != "$(read_value passes)" \
        || "$actual_failures" != "$(read_value failures)" \
        || "$actual_unsupported" != "$(read_value unsupported)" \
        || "$actual_skipped" != "$(read_value skipped)" \
        || "$actual_keys" != "$(read_value keys_sha256)" \
        || "$actual_nonpass" != "$(read_value nonpass_sha256)" \
        || "$actual_summary" != "$(read_value summary)" \
        || "$(sha256_file "$input_report")" != "$(read_value tsv_sha256)" \
        || "$(sha256_file "$input_json")" != "$(read_value jsonl_sha256)" ]]; then
        echo "error: global async Test262 classified vector drifted" >&2
        awk -F'\t' '
            !/^#/ && !($1 == "path" && $2 == "variant") && $7 != "pass" {
                print $1 "\t" $2 "\t" $7 "\t" $8 "\t" $9 "\t" $10
            }
        ' "$input_report" >&2
        exit 1
    fi
}

cd -- "$root"
suite=$("$script_dir/prepare-test262.sh")
source_dir=$(dirname -- "$suite")
tmp_dir=$(mktemp -d "${TMPDIR:-/tmp}/quickjs-oxide-r3al.XXXXXX")
trap 'rm -rf -- "$tmp_dir"' EXIT HUP INT TERM

metadata_records=$tmp_dir/metadata.records
metadata_tsv=$tmp_dir/metadata.tsv
selected_metadata=$tmp_dir/selected-metadata.tsv
positive=$tmp_dir/positive.txt
negative=$tmp_dir/negative.txt
async_paths=$tmp_dir/async.txt
sync_paths=$tmp_dir/sync.txt
double_mode=$tmp_dir/double-mode.txt
no_strict=$tmp_dir/no-strict.txt
only_strict=$tmp_dir/only-strict.txt
raw_paths=$tmp_dir/raw.txt
module_paths=$tmp_dir/module.txt
variant_keys=$tmp_dir/variant-keys.txt
feature_inventory=$tmp_dir/features.txt
include_inventory=$tmp_dir/includes.txt
flag_inventory=$tmp_dir/flags.txt
r3ak_before_keys=$tmp_dir/r3ak-before-keys.txt
r3ak_candidate_paths=$tmp_dir/r3ak-candidate-paths.txt
r3al_candidate_report=$tmp_dir/r3al-candidates.tsv
r3al_candidate_keys=$tmp_dir/r3al-candidate-keys.txt
transition_vector=$tmp_dir/r3ak-r3al-transitions.tsv
newly_runnable_keys=$tmp_dir/newly-runnable-keys.txt
repeat_report=$tmp_dir/repeat-8.tsv
alternate_report=$tmp_dir/alternate-5.tsv

expect_value quickjs 2026-06-04
expect_value test262 5c8206929d81b2d3d727ca6aac56c18358c8d790
expect_value test262_patch_sha256 f4b23b04641d438df0826fb17d7a5db276af2bdb085b42cc09aa8d50e0da9ba3
expect_value test262_config_sha256 79c64748ff1182baf5433d0a8378e3666738a785d02faf71f0d459ed42ae897b
expect_value test262_metadata_sha256 a37219960819e56a5c5c1723d31d6a33095c778bf5347385187fde96f927a06a
expect_value oxide_profile_sha256 fc6e8010c982bd6324b146e5f8e3ea0592aac7c03a323a8dbc8d778b4b670b23
expect_value schema test262-canonical-classified-v2
expect_value mode both
expect_value timeout_ms 30000
expect_value primary_workers 8
expect_value repeat_workers 8
expect_value alternate_workers 5
expect_value paths 3589
expect_value variants 7076
expect_value quickjs_passes 3589
expect_value runnable 7076
expect_value passes 7076
expect_value failures 0
expect_value unsupported 0
expect_value skipped 0
expect_value summary pass=7076
expect_value r3ak_candidate_paths 6496
expect_value r3ak_candidate_variants 12647
expect_value r3ak_unsupported_async 9992
expect_value r3ak_unsupported_feature 2580
expect_value r3ak_unsupported_module_async 75
expect_value newly_runnable 7076
expect_value r3al_candidate_unsupported 5571
expect_value r3al_candidate_summary "pass=7076 unsupported-feature=4540 unsupported-host-create-realm=2 unsupported-host-is-html-dda=2 unsupported-module=75 unsupported-negative-provenance=952"

if [[ "$(inventory_count "$manifest")" != "$(read_value paths)" \
    || "$(LC_ALL=C sort -u "$manifest" | inventory_count /dev/stdin)" != "$(read_value paths)" \
    || "$(sha256_file "$manifest")" != "$(read_value manifest_sha256)" ]]; then
    echo "error: global async manifest cardinality or content drifted" >&2
    exit 1
fi
LC_ALL=C sort -c "$manifest"
verify_r3ak_before

if [[ "$(sha256_file "$profile")" != "$(read_value oxide_profile_sha256)" ]]; then
    echo "error: global async capability profile drifted" >&2
    exit 1
fi
[[ "$(profile_section execution)" == "async=true" ]] \
    || { echo "error: global profile must enable only async execution" >&2; exit 1; }
if [[ "$(profile_section features | inventory_count /dev/stdin)" != "76" \
    || "$(profile_section features | grep -Ec '^async-(functions|iteration)$')" != "2" ]]; then
    echo "error: global profile async feature inventory drifted" >&2
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
awk -F'\t' '
    NR == FNR { selected[$1]=1; next }
    $1 in selected { print }
' "$manifest" "$metadata_tsv" > "$selected_metadata"
if [[ "$(inventory_count "$selected_metadata")" != "$(read_value paths)" \
    || "$(sha256_file "$selected_metadata")" != "$(read_value metadata_selection_sha256)" ]]; then
    echo "error: global async selected metadata drifted" >&2
    exit 1
fi

awk -F'\t' '$5 == "" { print $1 }' "$selected_metadata" > "$positive"
awk -F'\t' '$5 != "" { print $1 }' "$selected_metadata" > "$negative"
awk -F'\t' 'index("," $3 ",", ",async,") { print $1 }' \
    "$selected_metadata" > "$async_paths"
awk -F'\t' '!index("," $3 ",", ",async,") { print $1 }' \
    "$selected_metadata" > "$sync_paths"
awk -F'\t' '
    !index("," $3 ",", ",module,") &&
    !index("," $3 ",", ",noStrict,") &&
    !index("," $3 ",", ",onlyStrict,") &&
    !index("," $3 ",", ",raw,") { print $1 }
' "$selected_metadata" > "$double_mode"
awk -F'\t' 'index("," $3 ",", ",noStrict,") { print $1 }' \
    "$selected_metadata" > "$no_strict"
awk -F'\t' 'index("," $3 ",", ",onlyStrict,") { print $1 }' \
    "$selected_metadata" > "$only_strict"
awk -F'\t' 'index("," $3 ",", ",raw,") { print $1 }' \
    "$selected_metadata" > "$raw_paths"
awk -F'\t' 'index("," $3 ",", ",module,") { print $1 }' \
    "$selected_metadata" > "$module_paths"
awk -F'\t' '
    function has(list, value) {
        return index("," list ",", "," value ",") != 0
    }
    {
        if (has($3, "module") || has($3, "noStrict") || has($3, "raw")) {
            print $1 "\tsloppy"
        } else if (has($3, "onlyStrict")) {
            print $1 "\tstrict"
        } else {
            print $1 "\tsloppy"
            print $1 "\tstrict"
        }
    }
' "$selected_metadata" | LC_ALL=C sort > "$variant_keys"
awk -F'\t' '
    {
        count=split($4, values, ",")
        for (i=1; i<=count; i++) if (values[i] != "") print values[i]
    }
' "$selected_metadata" | LC_ALL=C sort -u > "$feature_inventory"
awk -F'\t' '
    {
        count=split($2, values, ",")
        for (i=1; i<=count; i++) if (values[i] != "") print values[i]
    }
' "$selected_metadata" | LC_ALL=C sort -u > "$include_inventory"
awk -F'\t' '
    {
        count=split($3, values, ",")
        for (i=1; i<=count; i++) if (values[i] != "") print values[i]
    }
' "$selected_metadata" | LC_ALL=C sort -u > "$flag_inventory"

verify_count positive_paths "$positive"
verify_count negative_paths "$negative"
verify_count async_paths "$async_paths"
verify_count sync_paths "$sync_paths"
verify_count double_mode_paths "$double_mode"
verify_count no_strict_paths "$no_strict"
verify_count only_strict_paths "$only_strict"
verify_count raw_paths "$raw_paths"
verify_count module_paths "$module_paths"
verify_count variants "$variant_keys"
verify_count features "$feature_inventory"
verify_count includes "$include_inventory"
verify_count flags "$flag_inventory"

if [[ "$(sha256_file "$variant_keys")" != "$(read_value keys_sha256)" \
    || "$(sha256_file "$feature_inventory")" != "$(read_value features_sha256)" \
    || "$(sha256_file "$include_inventory")" != "$(read_value includes_sha256)" \
    || "$(sha256_file "$flag_inventory")" != "$(read_value flags_sha256)" ]]; then
    echo "error: global async metadata inventory hash drifted" >&2
    exit 1
fi
if [[ -n "$(comm -23 "$feature_inventory" <(profile_section features))" ]]; then
    echo "error: global async manifest escaped globally admitted features" >&2
    exit 1
fi

verify_quickjs_oracle
if "$check_only"; then
    printf 'Global async inputs verified: %s paths / %s variants; pinned QuickJS passes all %s paths\n' \
        "$(read_value paths)" "$(read_value variants)" "$(read_value quickjs_passes)"
    exit 0
fi

r3al_candidate_output=$(run_r3al_candidates \
    "$(read_value primary_workers)" "$r3al_candidate_report")
printf '%s\n' "$r3al_candidate_output"
verify_transition_join "$r3al_candidate_report" "$r3al_candidate_output"

rm -f -- "$report" "$json_report"
primary_output=$(run_oxide "$(read_value primary_workers)" "$report")
printf '%s\n' "$primary_output"
verify_report "$report" "$json_report" "$primary_output"

repeat_output=$(run_oxide "$(read_value repeat_workers)" "$repeat_report")
printf '%s\n' "$repeat_output"
verify_report "$repeat_report" "${repeat_report%.tsv}.jsonl" "$repeat_output"

alternate_output=$(run_oxide "$(read_value alternate_workers)" "$alternate_report")
printf '%s\n' "$alternate_output"
verify_report "$alternate_report" "${alternate_report%.tsv}.jsonl" "$alternate_output"

if ! cmp -s "$report" "$repeat_report" \
    || ! cmp -s "$json_report" "${repeat_report%.tsv}.jsonl" \
    || ! cmp -s "$report" "$alternate_report" \
    || ! cmp -s "$json_report" "${alternate_report%.tsv}.jsonl"; then
    echo "error: global async 8/8/5 reports are not byte-identical" >&2
    exit 1
fi

printf 'Global async Test262 gate is deterministic: %s/%s pass across %s paths (workers %s/%s/%s); QuickJS %s/%s\n' \
    "$(read_value passes)" \
    "$(read_value variants)" \
    "$(read_value paths)" \
    "$(read_value primary_workers)" \
    "$(read_value repeat_workers)" \
    "$(read_value alternate_workers)" \
    "$(read_value quickjs_passes)" \
    "$(read_value paths)"
