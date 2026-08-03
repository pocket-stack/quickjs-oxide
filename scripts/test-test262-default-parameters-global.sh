#!/usr/bin/env bash
# Reproduce the exact global Test262 admission for default-parameters.

set -euo pipefail
export TZ=America/Los_Angeles
export LC_ALL=C

script_dir=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd)
root=$(CDPATH='' cd -- "$script_dir/.." && pwd)
baseline=tests/test262-default-parameters-global-baseline.txt
canonical_baseline=tests/test262-full-baseline.txt
focused_baseline=tests/test262-default-parameters-baseline.txt
focused_gate=scripts/test-test262-default-parameters.sh
parent_profile=tests/test262-default-parameters-parent.conf
focused_candidate_profile=tests/test262-default-parameters-candidate.conf
candidate_profile=tests/test262-default-parameters-global-candidate.conf
live_profile=compat/test262-oxide.conf
tag_manifest=tests/test262-default-parameters-universe.txt
companion_manifest=tests/test262-default-parameters-strict-body.txt
transition_receipt=tests/test262-default-parameters-global-transitions.tsv
parent_tag_report=target/test262-default-parameters-global-parent-tag.tsv
parent_tag_json_report=target/test262-default-parameters-global-parent-tag.jsonl
candidate_tag_report=target/test262-default-parameters-global-candidate-tag.tsv
candidate_tag_json_report=target/test262-default-parameters-global-candidate-tag.jsonl
parent_companion_report=target/test262-default-parameters-global-parent-companion.tsv
parent_companion_json_report=target/test262-default-parameters-global-parent-companion.jsonl
candidate_companion_report=target/test262-default-parameters-global-candidate-companion.tsv
candidate_companion_json_report=target/test262-default-parameters-global-candidate-companion.jsonl
parent_full_report=target/test262-default-parameters-global-parent-full.tsv
parent_full_json_report=target/test262-default-parameters-global-parent-full.jsonl
candidate_full_report=target/test262-default-parameters-global-candidate-full.tsv
candidate_full_json_report=target/test262-default-parameters-global-candidate-full.jsonl
workers=${TEST262_WORKERS:-8}
full_workers=${TEST262_FULL_WORKERS:-2}
lock_dir="$root/target/test262-default-parameters-global.lock"
lock_held=0
run_dir=
runner=
metadata_tmp=
metadata_tsv_tmp=
derived_paths_tmp=
derived_keys_tmp=
tag_negatives_tmp=
companion_metadata_tmp=
companion_parent_tmp=
companion_added_tmp=
companion_keys_tmp=
companion_added_keys_tmp=
combined_paths_tmp=
combined_keys_tmp=
companion_worker_tmp=
companion_quickjs_tmp=
parent_combined_tmp=
candidate_combined_tmp=
parent_combined_json_tmp=
candidate_combined_json_tmp=
transition_tmp=
changed_keys_tmp=
schema_expected_tmp=
schema_actual_tmp=
live_validation_report=
live_validation_json_report=

cleanup() {
    [[ -z "$metadata_tmp" ]] || rm -f -- "$metadata_tmp"
    [[ -z "$metadata_tsv_tmp" ]] || rm -f -- "$metadata_tsv_tmp"
    [[ -z "$derived_paths_tmp" ]] || rm -f -- "$derived_paths_tmp"
    [[ -z "$derived_keys_tmp" ]] || rm -f -- "$derived_keys_tmp"
    [[ -z "$tag_negatives_tmp" ]] || rm -f -- "$tag_negatives_tmp"
    [[ -z "$companion_metadata_tmp" ]] || rm -f -- "$companion_metadata_tmp"
    [[ -z "$companion_parent_tmp" ]] || rm -f -- "$companion_parent_tmp"
    [[ -z "$companion_added_tmp" ]] || rm -f -- "$companion_added_tmp"
    [[ -z "$companion_keys_tmp" ]] || rm -f -- "$companion_keys_tmp"
    [[ -z "$companion_added_keys_tmp" ]] || rm -f -- "$companion_added_keys_tmp"
    [[ -z "$combined_paths_tmp" ]] || rm -f -- "$combined_paths_tmp"
    [[ -z "$combined_keys_tmp" ]] || rm -f -- "$combined_keys_tmp"
    [[ -z "$companion_worker_tmp" ]] || rm -f -- "$companion_worker_tmp"
    [[ -z "$companion_quickjs_tmp" ]] || rm -f -- "$companion_quickjs_tmp"
    [[ -z "$parent_combined_tmp" ]] || rm -f -- "$parent_combined_tmp"
    [[ -z "$candidate_combined_tmp" ]] || rm -f -- "$candidate_combined_tmp"
    [[ -z "$parent_combined_json_tmp" ]] \
        || rm -f -- "$parent_combined_json_tmp"
    [[ -z "$candidate_combined_json_tmp" ]] \
        || rm -f -- "$candidate_combined_json_tmp"
    [[ -z "$transition_tmp" ]] || rm -f -- "$transition_tmp"
    [[ -z "$changed_keys_tmp" ]] || rm -f -- "$changed_keys_tmp"
    [[ -z "$schema_expected_tmp" ]] || rm -f -- "$schema_expected_tmp"
    [[ -z "$schema_actual_tmp" ]] || rm -f -- "$schema_actual_tmp"
    [[ -z "$live_validation_report" ]] || rm -f -- "$live_validation_report"
    [[ -z "$live_validation_json_report" ]] \
        || rm -f -- "$live_validation_json_report"
    [[ -z "$runner" ]] || rm -f -- "$runner"
    [[ -z "$run_dir" ]] || rmdir -- "$run_dir" 2>/dev/null || true
    if [[ "$lock_held" == 1 ]]; then
        rm -f -- "$lock_dir/pid"
        rmdir -- "$lock_dir" 2>/dev/null || true
    fi
}
trap cleanup EXIT

usage() {
    printf 'usage: %s [--check|--full|--bless-tag|--bless-full]\n' "${0##*/}"
    printf '  --check       verify pinned metadata, profiles, manifests, and focused gate\n'
    printf '  --full        verify tag receipts plus the exact whole-suite join\n'
    printf '  --bless-tag   intentionally replace tag hashes and transition receipt\n'
    printf '  --bless-full  strict alias of --full once the receipt is frozen\n'
}

mode=tag
case ${1-} in
    '') ;;
    --check) mode=check ;;
    --full) mode=full ;;
    --bless-tag) mode=bless-tag ;;
    --bless-full) mode=bless-full ;;
    -h | --help) usage; exit 0 ;;
    *) usage >&2; exit 2 ;;
esac
[[ $# -le 1 ]] || { usage >&2; exit 2; }
[[ "$workers" =~ ^[1-9][0-9]*$ ]] || {
    echo 'error: TEST262_WORKERS must be a positive integer' >&2
    exit 2
}
[[ "$full_workers" =~ ^[1-9][0-9]*$ ]] || {
    echo 'error: TEST262_FULL_WORKERS must be a positive integer' >&2
    exit 2
}

read_value_from() {
    local file=$1 key=$2
    awk -F= -v key="$key" '
        $1 == key {
            if (found++) exit 2
            sub(/^[^=]*=/, "")
            print
        }
        END { if (found != 1) exit 1 }
    ' "$file"
}

read_value() {
    read_value_from "$baseline" "$1"
}

expected_baseline_keys() {
    printf '%s\n' \
        quickjs test262 test262_patch_sha256 test262_config_sha256 \
        test262_metadata_records test262_metadata_sha256 schema mode timeout_ms \
        focused_baseline focused_baseline_sha256 \
        parent_oxide_profile_sha256 candidate_oxide_profile_sha256 \
        parent_features parent_features_sha256 candidate_features \
        candidate_features_sha256 added_features added_features_sha256 \
        parent_audited_negative_tests parent_audited_negative_tests_sha256 \
        candidate_audited_negative_tests candidate_audited_negative_tests_sha256 \
        added_audited_negative_tests added_audited_negative_tests_sha256 \
        tag_added_negative_tests tag_added_negative_tests_sha256 \
        companion_added_negative_tests companion_added_negative_tests_sha256 \
        execution_entries execution_sha256 tag_manifest tag_paths tag_paths_sha256 \
        tag_variants tag_keys_sha256 companion_manifest companion_paths \
        companion_paths_sha256 companion_variants companion_keys_sha256 \
        companion_metadata_sha256 companion_parent_audited_paths \
        companion_parent_audited_sha256 companion_added_paths \
        companion_added_paths_sha256 companion_added_variants \
        companion_added_keys_sha256 combined_paths combined_paths_sha256 \
        combined_variants combined_keys_sha256 companion_worker_variants \
        companion_worker_receipt_sha256 companion_quickjs_paths \
        companion_quickjs_variants companion_quickjs_log_sha256 \
        transition_receipt transition_rows \
        transition_receipt_sha256 transition_data_sha256 parent_tag_runnable \
        parent_tag_summary parent_tag_nonpass_sha256 parent_tag_tsv_sha256 \
        parent_tag_jsonl_sha256 candidate_tag_runnable candidate_tag_summary \
        candidate_tag_nonpass_sha256 candidate_tag_tsv_sha256 \
        candidate_tag_jsonl_sha256 parent_companion_runnable \
        parent_companion_summary parent_companion_nonpass_sha256 \
        parent_companion_tsv_sha256 parent_companion_jsonl_sha256 \
        candidate_companion_runnable candidate_companion_summary \
        candidate_companion_nonpass_sha256 candidate_companion_tsv_sha256 \
        candidate_companion_jsonl_sha256 parent_combined_runnable \
        parent_combined_summary parent_combined_tsv_data_sha256 \
        parent_combined_jsonl_data_sha256 candidate_combined_runnable \
        candidate_combined_summary candidate_combined_tsv_data_sha256 \
        candidate_combined_jsonl_data_sha256 transition_changed_rows \
        transition_outcome_changed_rows transition_detail_only_rows \
        transition_unchanged_rows full_receipt_state full_variants \
        full_keys_sha256 full_combined_rows full_noncombined_rows full_changed_rows \
        full_outcome_changed_rows full_detail_only_rows full_unchanged_rows \
        previous_pass_regressions parent_full_runnable parent_full_passes \
        parent_full_unsupported_feature parent_full_unsupported_negative_provenance \
        parent_full_total_unsupported \
        parent_full_summary parent_full_tsv_sha256 parent_full_jsonl_sha256 \
        candidate_full_runnable candidate_full_passes \
        candidate_full_unsupported_feature \
        candidate_full_unsupported_negative_provenance \
        candidate_full_total_unsupported \
        candidate_full_summary candidate_full_tsv_sha256 \
        candidate_full_jsonl_sha256 full_parent_combined_tsv_data_sha256 \
        full_parent_combined_jsonl_data_sha256 \
        full_candidate_combined_tsv_data_sha256 \
        full_candidate_combined_jsonl_data_sha256 \
        full_noncombined_tsv_data_sha256 full_noncombined_jsonl_data_sha256
}

expected_canonical_baseline_keys() {
    printf '%s\n' schema timeout_ms variants runnable passes tsv_sha256 \
        jsonl_sha256 summary
}

validate_key_value_schema() {
    local file=$1 expected_keys=$2
    schema_expected_tmp=$(mktemp target/test262-global-schema-expected.XXXXXX)
    schema_actual_tmp=$(mktemp target/test262-global-schema-actual.XXXXXX)
    "$expected_keys" | LC_ALL=C sort >"$schema_expected_tmp"
    if ! awk -F= -v file="$file" '
            /^[[:space:]]*$/ || /^[[:space:]]*#/ { next }
            index($0, "=") <= 1 {
                printf "error: malformed baseline entry in %s: %s\n", file, $0 \
                    >"/dev/stderr"
                exit 2
            }
            {
                key=$1
                if (key ~ /[[:space:]]/ || seen[key]++) {
                    printf "error: duplicate or malformed baseline key in %s: %s\n", \
                        file, key >"/dev/stderr"
                    exit 3
                }
                print key
            }
        ' "$file" | LC_ALL=C sort >"$schema_actual_tmp"; then
        return 1
    fi
    diff -u "$schema_expected_tmp" "$schema_actual_tmp"
    rm -f -- "$schema_expected_tmp" "$schema_actual_tmp"
    schema_expected_tmp=
    schema_actual_tmp=
}

validate_baseline_schema() {
    validate_key_value_schema "$baseline" expected_baseline_keys
}

expect_value() {
    local key=$1 expected=$2 actual
    actual=$(read_value "$key")
    [[ "$actual" == "$expected" ]] || {
        printf 'error: default-parameters global baseline drifted for %s: %s != %s\n' \
            "$key" "$actual" "$expected" >&2
        exit 1
    }
}

sha256_file() {
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum "$1" | awk '{ print $1 }'
    else
        shasum -a 256 "$1" | awk '{ print $1 }'
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
    local profile=$1 section=$2
    awk -v section="[$section]" '
        $0 == section { inside=1; next }
        /^\[/ { inside=0 }
        inside && NF && $1 !~ /^[#;]/ { print }
    ' "$profile"
}

manifest_paths() {
    awk 'NF && $1 !~ /^#/ { print }' "$1"
}

read_header() {
    local report=$1 key=$2
    awk -F= -v key="# $key" '
        $1 == key {
            if (found++) exit 2
            sub(/^[^=]*=/, "")
            print
        }
        END { if (found != 1) exit 1 }
    ' "$report"
}

report_rows() {
    awk -F'\t' '!/^#/ && !($1 == "path" && $2 == "variant") { print }' "$1"
}

report_keys() {
    report_rows "$1" | awk -F'\t' '{ print $1 "\t" $2 }' | LC_ALL=C sort
}

rows_for_paths() {
    local manifest=$1 report=$2
    awk -F'\t' '
        NR == FNR {
            if (NF && $1 !~ /^#/) wanted[$0]=1
            next
        }
        !/^#/ && !($1 == "path" && $2 == "variant") && ($1 in wanted) {
            print
        }
    ' "$manifest" "$report"
}

rows_without_paths() {
    local manifest=$1 report=$2
    awk -F'\t' '
        NR == FNR {
            if (NF && $1 !~ /^#/) blocked[$0]=1
            next
        }
        !/^#/ && !($1 == "path" && $2 == "variant") && !($1 in blocked) {
            print
        }
    ' "$manifest" "$report"
}

json_rows_for_paths() {
    local manifest=$1 report=$2
    awk '
        NR == FNR {
            if (NF && $1 !~ /^#/) wanted[$0]=1
            next
        }
        /^\{"kind":"result"/ {
            if (!match($0, /"path":"[^"]*"/)) exit 2
            path=substr($0, RSTART + 8, RLENGTH - 9)
            if (path in wanted) print
        }
    ' "$manifest" "$report"
}

json_rows_without_paths() {
    local manifest=$1 report=$2
    awk '
        NR == FNR {
            if (NF && $1 !~ /^#/) blocked[$0]=1
            next
        }
        /^\{"kind":"result"/ {
            if (!match($0, /"path":"[^"]*"/)) exit 2
            path=substr($0, RSTART + 8, RLENGTH - 9)
            if (!(path in blocked)) print
        }
    ' "$manifest" "$report"
}

json_report_keys() {
    local report=$1
    awk -v report="$report" '
        function fail(message) {
            printf "error: JSONL report %s: %s\n", report, message >"/dev/stderr"
            failed=1
            exit 2
        }
        /^\{"kind":"metadata",/ {
            metadata++
            if (NR != 1) fail("metadata record is not first")
            next
        }
        /^\{"kind":"result",/ {
            results++
            if (!match($0, /"path":"[^"]*"/)) fail("result has no path")
            path=substr($0, RSTART + 8, RLENGTH - 9)
            if (!match($0, /"variant":"[^"]*"/)) fail("result has no variant")
            variant=substr($0, RSTART + 11, RLENGTH - 12)
            key=path "\t" variant
            if (seen[key]++) fail("duplicate result key")
            print key
            next
        }
        /^\{"kind":"summary",/ {
            summary++
            summary_line=NR
            next
        }
        { fail("unexpected record") }
        END {
            if (!failed && metadata != 1) fail("expected one metadata record")
            if (!failed && summary != 1) fail("expected one summary record")
            if (!failed && summary_line != NR) fail("summary record is not last")
        }
    ' "$report" | LC_ALL=C sort
}

report_summary() {
    tail -n 1 "$1" | sed 's/^# summary //'
}

json_report_summary() {
    tail -n 1 "$1" | awk '
        /^\{"kind":"summary","outcomes":\{.*\}\}$/ {
            sub(/^\{"kind":"summary","outcomes":\{/, "")
            sub(/\}\}$/, "")
            gsub(/":/, "=")
            gsub(/"/, "")
            gsub(/,/, " ")
            print
            found=1
        }
        END { if (!found) exit 1 }
    '
}

expected_json_metadata() {
    local profile=$1
    printf '{"kind":"metadata","schema":2,"quickjs":"%s","test262":"%s","test262_patch_sha256":"%s","test262_config_sha256":"%s","test262_metadata_sha256":"%s","oxide_profile_sha256":"%s","profile":"%s","mode":"%s"}\n' \
        "$(read_value quickjs)" \
        "$(read_value test262)" \
        "$(read_value test262_patch_sha256)" \
        "$(read_value test262_config_sha256)" \
        "$(read_value test262_metadata_sha256)" \
        "$profile" \
        "$(read_value schema)" \
        "$(read_value mode)"
}

verify_report() {
    local report=$1 json_report=$2 profile=$3 rows=$4 summary=$5
    local tsv_keys json_keys
    [[ -f "$report" && -f "$json_report" \
        && "$(read_header "$report" quickjs)" == "$(read_value quickjs)" \
        && "$(read_header "$report" test262)" == "$(read_value test262)" \
        && "$(read_header "$report" test262_patch_sha256)" \
            == "$(read_value test262_patch_sha256)" \
        && "$(read_header "$report" test262_config_sha256)" \
            == "$(read_value test262_config_sha256)" \
        && "$(read_header "$report" test262_metadata_sha256)" \
            == "$(read_value test262_metadata_sha256)" \
        && "$(read_header "$report" oxide_profile_sha256)" == "$profile" \
        && "$(read_header "$report" profile)" == "$(read_value schema)" \
        && "$(read_header "$report" mode)" == "$(read_value mode)" \
        && "$(report_rows "$report" | wc -l | tr -d '[:space:]')" == "$rows" \
        && "$(report_summary "$report")" == "$summary" \
        && "$(head -n 1 "$json_report")" == "$(expected_json_metadata "$profile")" \
        && "$(json_report_summary "$json_report")" == "$summary" ]] || {
        printf 'error: report metadata or summary drifted: %s\n' "$report" >&2
        exit 1
    }
    tsv_keys=$(report_keys "$report")
    json_keys=$(json_report_keys "$json_report")
    diff -u <(printf '%s\n' "$tsv_keys") <(printf '%s\n' "$json_keys")
}

execution_runnable() {
    printf '%s\n' "$1" | awk '
        /^execution: runnable=/ {
            sub(/^execution: runnable=/, "")
            sub(/ .*/, "")
            print
            found=1
        }
        END { if (!found) exit 1 }
    '
}

report_outcome_count() {
    local report=$1 outcome=$2
    report_rows "$report" | awk -F'\t' -v outcome="$outcome" '
        $7 == outcome { count++ }
        END { print count + 0 }
    '
}

report_nonpass_sha256() {
    report_rows "$1" | awk -F'\t' '$7 != "pass" { print }' | sha256_stream
}

unsupported_total() {
    printf '%s\n' "$1" | awk '
        {
            for (i=1; i<=NF; i++) {
                split($i, pair, "=")
                if (pair[1] ~ /^unsupported-/) total+=pair[2]
            }
        }
        END { print total + 0 }
    '
}

update_values() {
    local file=$1 updates_tmp output_tmp entry
    shift
    updates_tmp=$(mktemp "$file.updates.XXXXXX")
    output_tmp=$(mktemp "$file.output.XXXXXX")
    for entry in "$@"; do printf '%s\n' "$entry"; done >"$updates_tmp"
    awk -F= '
        NR == FNR {
            key=$1
            sub(/^[^=]*=/, "")
            replacement[key]=$0
            next
        }
        $1 in replacement {
            print $1 "=" replacement[$1]
            seen[$1]=1
            next
        }
        { print }
        END {
            for (key in replacement) if (!(key in seen)) {
                print "missing baseline key: " key >"/dev/stderr"
                bad=1
            }
            if (bad) exit 1
        }
    ' "$updates_tmp" "$file" >"$output_tmp"
    chmod 644 "$output_tmp"
    mv -- "$output_tmp" "$file"
    rm -f -- "$updates_tmp"
}

run_tag() {
    local profile=$1 report=$2
    "$runner" \
        --suite "$suite" \
        --config "$source_dir/test262.conf" \
        --oxide-profile "$profile" \
        --manifest "$tag_manifest" \
        --report "$report" \
        --mode "$(read_value mode)" \
        --workers "$workers" \
        --timeout-ms "$(read_value timeout_ms)" \
        --allow-failures
}

run_companion() {
    local profile=$1 report=$2
    "$runner" \
        --suite "$suite" \
        --config "$source_dir/test262.conf" \
        --oxide-profile "$profile" \
        --manifest "$companion_manifest" \
        --report "$report" \
        --mode "$(read_value mode)" \
        --workers "$workers" \
        --timeout-ms "$(read_value timeout_ms)" \
        --allow-failures
}

run_full() {
    local profile=$1 report=$2
    "$runner" \
        --suite "$suite" \
        --config "$source_dir/test262.conf" \
        --oxide-profile "$profile" \
        --all \
        --report "$report" \
        --mode "$(read_value mode)" \
        --workers "$full_workers" \
        --timeout-ms "$(read_value timeout_ms)" \
        --allow-failures
}

cd -- "$root"
mkdir -p target
if ! mkdir "$lock_dir" 2>/dev/null; then
    printf 'error: default-parameters global gate is already running: %s\n' \
        "$lock_dir" >&2
    exit 1
fi
lock_held=1
printf '%s\n' "$$" >"$lock_dir/pid"
validate_baseline_schema
run_dir=$(mktemp -d target/test262-default-parameters-global-run.XXXXXX)
cargo build --locked --release --quiet --bin run-test262
runner="$run_dir/run-test262"
cp target/release/run-test262 "$runner"
chmod 755 "$runner"
suite=$("$script_dir/prepare-test262.sh")
source_dir=$(dirname -- "$suite")

expect_value quickjs 2026-06-04
expect_value test262 5c8206929d81b2d3d727ca6aac56c18358c8d790
expect_value test262_patch_sha256 f4b23b04641d438df0826fb17d7a5db276af2bdb085b42cc09aa8d50e0da9ba3
expect_value test262_config_sha256 79c64748ff1182baf5433d0a8378e3666738a785d02faf71f0d459ed42ae897b
expect_value test262_metadata_records 53125
expect_value test262_metadata_sha256 a37219960819e56a5c5c1723d31d6a33095c778bf5347385187fde96f927a06a
expect_value schema test262-canonical-classified-v2
expect_value mode both
expect_value timeout_ms 30000
expect_value focused_baseline tests/test262-default-parameters-baseline.txt
expect_value parent_oxide_profile_sha256 d55e0625b1f6878b7afa6885d82cf332909271ce1c2222100fe3a403a8455969
expect_value candidate_oxide_profile_sha256 63f139b1a74da9a6114180593770dbcc86bb84fbafab5731f59e1387175c5a6a
expect_value parent_features 92
expect_value candidate_features 93
expect_value added_features 1
expect_value parent_audited_negative_tests 924
expect_value candidate_audited_negative_tests 1154
expect_value added_audited_negative_tests 230
expect_value tag_added_negative_tests 219
expect_value companion_added_negative_tests 11
expect_value execution_entries 1
expect_value tag_paths 2269
expect_value tag_variants 4516
expect_value companion_paths 14
expect_value companion_variants 28
expect_value companion_parent_audited_paths 3
expect_value companion_added_paths 11
expect_value companion_added_variants 22
expect_value combined_paths 2283
expect_value combined_variants 4544
expect_value companion_worker_variants 28
expect_value companion_quickjs_paths 14
expect_value companion_quickjs_variants 28
expect_value transition_rows 4544
expect_value transition_changed_rows 4536
expect_value transition_outcome_changed_rows 3374
expect_value transition_detail_only_rows 1162
expect_value transition_unchanged_rows 8
expect_value parent_tag_runnable 0
expect_value parent_tag_summary 'unsupported-feature=4514 unsupported-host-is-html-dda=2'
expect_value candidate_tag_runnable 3352
expect_value candidate_tag_summary 'pass=3352 unsupported-feature=1162 unsupported-host-is-html-dda=2'
expect_value parent_companion_runnable 6
expect_value parent_companion_summary 'pass=6 unsupported-negative-provenance=22'
expect_value candidate_companion_runnable 28
expect_value candidate_companion_summary pass=28
expect_value parent_combined_runnable 6
expect_value parent_combined_summary 'pass=6 unsupported-feature=4514 unsupported-host-is-html-dda=2 unsupported-negative-provenance=22'
expect_value candidate_combined_runnable 3380
expect_value candidate_combined_summary 'pass=3380 unsupported-feature=1162 unsupported-host-is-html-dda=2'
expect_value full_receipt_state frozen
expect_value full_variants 102037
expect_value full_combined_rows 4544
expect_value full_noncombined_rows 97493
expect_value full_changed_rows 4536
expect_value full_outcome_changed_rows 3374
expect_value full_detail_only_rows 1162
expect_value full_unchanged_rows 97501
expect_value previous_pass_regressions 0
expect_value parent_full_runnable 60218
expect_value parent_full_passes 59699
expect_value parent_full_unsupported_feature 18426
expect_value parent_full_unsupported_negative_provenance 3473
expect_value parent_full_total_unsupported 23393
expect_value candidate_full_runnable 63592
expect_value candidate_full_passes 63073
expect_value candidate_full_unsupported_feature 15074
expect_value candidate_full_unsupported_negative_provenance 3451
expect_value candidate_full_total_unsupported 20019

if [[ "$mode" == bless-full ]]; then
    # Once the independently reproduced projected vector is frozen, the
    # blessing spelling is a strict replay and cannot rewrite it.
    mode=full
fi

for required in "$baseline" "$canonical_baseline" "$focused_baseline" \
    "$focused_gate" "$parent_profile" "$focused_candidate_profile" \
    "$candidate_profile" "$live_profile" "$tag_manifest" \
    "$companion_manifest" "$transition_receipt"; do
    [[ -e "$required" ]] || {
        printf 'error: missing default-parameters global asset: %s\n' \
            "$required" >&2
        exit 1
    }
done
[[ -x "$focused_gate" ]] || {
    echo 'error: focused default-parameters gate is not executable' >&2
    exit 1
}
validate_key_value_schema "$canonical_baseline" expected_canonical_baseline_keys
for binding in schema:schema timeout_ms:timeout_ms full_variants:variants; do
    global_key=${binding%%:*}
    canonical_key=${binding#*:}
    [[ "$(read_value "$global_key")" \
            == "$(read_value_from "$canonical_baseline" "$canonical_key")" ]] || {
        printf 'error: canonical metadata binding drifted: %s\n' \
            "$global_key" >&2
        exit 1
    }
done

canonical_vector_matches() {
    local state=$1 prefix runnable passes tsv jsonl summary
    prefix=${state}_full
    runnable=$(read_value "${prefix}_runnable")
    passes=$(read_value "${prefix}_passes")
    tsv=$(read_value "${prefix}_tsv_sha256")
    jsonl=$(read_value "${prefix}_jsonl_sha256")
    summary=$(read_value "${prefix}_summary")
    [[ "$(read_value_from "$canonical_baseline" runnable)" == "$runnable" \
        && "$(read_value_from "$canonical_baseline" passes)" == "$passes" \
        && "$(read_value_from "$canonical_baseline" tsv_sha256)" == "$tsv" \
        && "$(read_value_from "$canonical_baseline" jsonl_sha256)" == "$jsonl" \
        && "$(read_value_from "$canonical_baseline" summary)" == "$summary" ]]
}

# Authenticate the exact one-feature plus 230-negative profile delta. The
# focused profile contributes 219 tagged negatives; the strict-body companion
# contributes the remaining 11 paths.
live_profile_sha256_before=$(sha256_file "$live_profile")
parent_features=$(profile_section "$parent_profile" features)
focused_features=$(profile_section "$focused_candidate_profile" features)
candidate_features=$(profile_section "$candidate_profile" features)
parent_negatives=$(profile_section "$parent_profile" audited-negative-tests)
focused_negatives=$(profile_section "$focused_candidate_profile" audited-negative-tests)
candidate_negatives=$(profile_section "$candidate_profile" audited-negative-tests)
parent_execution=$(profile_section "$parent_profile" execution)
candidate_execution=$(profile_section "$candidate_profile" execution)
live_features=$(profile_section "$live_profile" features)
live_negatives=$(profile_section "$live_profile" audited-negative-tests)
live_execution=$(profile_section "$live_profile" execution)
for entries in "$parent_features" "$focused_features" "$candidate_features" \
    "$parent_negatives" "$focused_negatives" "$candidate_negatives" \
    "$live_features" "$live_negatives"; do
    printf '%s\n' "$entries" | LC_ALL=C sort -cu
done
added_features=$(comm -13 <(printf '%s\n' "$parent_features") \
    <(printf '%s\n' "$candidate_features"))
removed_features=$(comm -23 <(printf '%s\n' "$parent_features") \
    <(printf '%s\n' "$candidate_features"))
tag_added_negatives=$(comm -13 <(printf '%s\n' "$parent_negatives") \
    <(printf '%s\n' "$focused_negatives"))
companion_added_negatives=$(comm -13 <(printf '%s\n' "$focused_negatives") \
    <(printf '%s\n' "$candidate_negatives"))
added_negatives=$(comm -13 <(printf '%s\n' "$parent_negatives") \
    <(printf '%s\n' "$candidate_negatives"))
removed_negatives=$(comm -23 <(printf '%s\n' "$parent_negatives") \
    <(printf '%s\n' "$candidate_negatives"))
[[ "$(sha256_file "$parent_profile")" \
        == "$(read_value parent_oxide_profile_sha256)" \
    && "$(sha256_file "$candidate_profile")" \
        == "$(read_value candidate_oxide_profile_sha256)" \
    && "$(printf '%s\n' "$parent_features" | wc -l | tr -d '[:space:]')" \
        == "$(read_value parent_features)" \
    && "$(printf '%s\n' "$parent_features" | sha256_stream)" \
        == "$(read_value parent_features_sha256)" \
    && "$focused_features" == "$candidate_features" \
    && "$(printf '%s\n' "$candidate_features" | wc -l | tr -d '[:space:]')" \
        == "$(read_value candidate_features)" \
    && "$(printf '%s\n' "$candidate_features" | sha256_stream)" \
        == "$(read_value candidate_features_sha256)" \
    && "$added_features" == default-parameters \
    && -z "$removed_features" \
    && "$(printf '%s\n' "$added_features" | sha256_stream)" \
        == "$(read_value added_features_sha256)" \
    && "$(printf '%s\n' "$parent_negatives" | wc -l | tr -d '[:space:]')" \
        == "$(read_value parent_audited_negative_tests)" \
    && "$(printf '%s\n' "$parent_negatives" | sha256_stream)" \
        == "$(read_value parent_audited_negative_tests_sha256)" \
    && "$(printf '%s\n' "$candidate_negatives" | wc -l | tr -d '[:space:]')" \
        == "$(read_value candidate_audited_negative_tests)" \
    && "$(printf '%s\n' "$candidate_negatives" | sha256_stream)" \
        == "$(read_value candidate_audited_negative_tests_sha256)" \
    && "$(printf '%s\n' "$added_negatives" | wc -l | tr -d '[:space:]')" \
        == "$(read_value added_audited_negative_tests)" \
    && "$(printf '%s\n' "$added_negatives" | sha256_stream)" \
        == "$(read_value added_audited_negative_tests_sha256)" \
    && "$(printf '%s\n' "$tag_added_negatives" | wc -l | tr -d '[:space:]')" \
        == "$(read_value tag_added_negative_tests)" \
    && "$(printf '%s\n' "$tag_added_negatives" | sha256_stream)" \
        == "$(read_value tag_added_negative_tests_sha256)" \
    && "$(printf '%s\n' "$companion_added_negatives" | wc -l | tr -d '[:space:]')" \
        == "$(read_value companion_added_negative_tests)" \
    && "$(printf '%s\n' "$companion_added_negatives" | sha256_stream)" \
        == "$(read_value companion_added_negative_tests_sha256)" \
    && -z "$removed_negatives" \
    && "$parent_execution" == async=true \
    && "$candidate_execution" == "$parent_execution" \
    && "$(printf '%s\n' "$candidate_execution" | wc -l | tr -d '[:space:]')" \
        == "$(read_value execution_entries)" \
    && "$(printf '%s\n' "$candidate_execution" | sha256_stream)" \
        == "$(read_value execution_sha256)" ]] || {
    echo 'error: default-parameters global profile delta drifted' >&2
    exit 1
}
[[ -z "$(comm -23 <(printf '%s\n' "$candidate_features") \
        <(printf '%s\n' "$live_features"))" \
    && -z "$(comm -23 <(printf '%s\n' "$candidate_negatives") \
        <(printf '%s\n' "$live_negatives"))" \
    && "$candidate_execution" == "$live_execution" ]] || {
    echo 'error: live profile removed a default-parameters candidate capability' >&2
    exit 1
}
live_profile_sha256=$(sha256_file "$live_profile")
[[ "$live_profile_sha256" == "$live_profile_sha256_before" ]] || {
    echo 'error: live profile changed while the gate authenticated it' >&2
    exit 1
}
upstream_profile=$(awk -F'"' '
    $1 ~ /^oxide_profile_sha256 = / { print $2; found++ }
    END { if (found != 1) exit 1 }
' compat/upstream.toml)
[[ "$upstream_profile" == "$live_profile_sha256" ]] || {
    echo 'error: compat/upstream.toml does not authenticate the live profile' >&2
    exit 1
}
live_added_features=$(comm -13 <(printf '%s\n' "$candidate_features") \
    <(printf '%s\n' "$live_features"))
live_added_negatives=$(comm -13 <(printf '%s\n' "$candidate_negatives") \
    <(printf '%s\n' "$live_negatives"))
if [[ "$live_profile_sha256" == "$(read_value candidate_oxide_profile_sha256)" ]]; then
    canonical_vector_matches candidate || {
        echo 'error: live candidate requires its exact canonical full vector' >&2
        exit 1
    }
elif [[ -n "$live_added_features" || -n "$live_added_negatives" ]]; then
    : # A semantic descendant owns an independent current canonical vector.
else
    echo 'error: live profile differs without advancing candidate capabilities' >&2
    exit 1
fi

# Rebuild the pinned metadata inventories rather than trusting checked-in path
# lists alone.
metadata_tmp=$(mktemp target/test262-default-parameters-global-metadata.XXXXXX)
metadata_tsv_tmp=$(mktemp target/test262-default-parameters-global-metadata-tsv.XXXXXX)
derived_paths_tmp=$(mktemp target/test262-default-parameters-global-tag-paths.XXXXXX)
derived_keys_tmp=$(mktemp target/test262-default-parameters-global-tag-keys.XXXXXX)
tag_negatives_tmp=$(mktemp target/test262-default-parameters-global-tag-negatives.XXXXXX)
"$runner" --suite "$suite" --validate-metadata "$metadata_tmp"
tr '\0' '\t' <"$metadata_tmp" >"$metadata_tsv_tmp"
[[ "$(wc -l <"$metadata_tmp" | tr -d '[:space:]')" \
        == "$(read_value test262_metadata_records)" \
    && "$(sha256_file "$metadata_tmp")" \
        == "$(read_value test262_metadata_sha256)" \
    && "$(sha256_file "$source_dir/test262.conf")" \
        == "$(read_value test262_config_sha256)" ]] || {
    echo 'error: pinned metadata or QuickJS config drifted' >&2
    exit 1
}
awk -F'\t' -v paths="$derived_paths_tmp" -v keys="$derived_keys_tmp" \
    -v negatives="$tag_negatives_tmp" '
    function has(list, value) {
        return index("," list ",", "," value ",") != 0
    }
    function emit_keys(path, flags) {
        if (has(flags, "onlyStrict")) print path "\tstrict" >keys
        else if (has(flags, "noStrict")) print path "\tsloppy" >keys
        else {
            print path "\tsloppy" >keys
            print path "\tstrict" >keys
        }
    }
    !has($4, "default-parameters") { next }
    {
        if (has($3, "module") || has($3, "raw") ||
            (has($3, "onlyStrict") && has($3, "noStrict"))) bad=1
        if ($5 != "") {
            if ($5 != "parse" || $6 != "SyntaxError") bad=1
            print $1 >negatives
        } else if ($6 != "") bad=1
        print $1 >paths
        emit_keys($1, $3)
    }
    END { if (bad) exit 1 }
' "$metadata_tsv_tmp" || {
    echo 'error: default-parameters tag metadata contract drifted' >&2
    exit 1
}
for sorted in "$derived_paths_tmp" "$derived_keys_tmp" "$tag_negatives_tmp"; do
    LC_ALL=C sort -c "$sorted"
    [[ -z "$(uniq -d "$sorted")" ]] || {
        printf 'error: duplicate derived inventory entry: %s\n' "$sorted" >&2
        exit 1
    }
done
diff -u <(manifest_paths "$tag_manifest") "$derived_paths_tmp"
diff -u <(printf '%s\n' "$tag_added_negatives") "$tag_negatives_tmp"
[[ "$(read_value tag_manifest)" == "$tag_manifest" \
    && "$(wc -l <"$derived_paths_tmp" | tr -d '[:space:]')" \
        == "$(read_value tag_paths)" \
    && "$(sha256_file "$derived_paths_tmp")" == "$(read_value tag_paths_sha256)" \
    && "$(wc -l <"$derived_keys_tmp" | tr -d '[:space:]')" \
        == "$(read_value tag_variants)" \
    && "$(sha256_file "$derived_keys_tmp")" == "$(read_value tag_keys_sha256)" ]] || {
    echo 'error: default-parameters tag inventory drifted' >&2
    exit 1
}

companion_metadata_tmp=$(mktemp target/test262-default-parameters-global-companion-metadata.XXXXXX)
companion_parent_tmp=$(mktemp target/test262-default-parameters-global-companion-parent.XXXXXX)
companion_added_tmp=$(mktemp target/test262-default-parameters-global-companion-added.XXXXXX)
companion_keys_tmp=$(mktemp target/test262-default-parameters-global-companion-keys.XXXXXX)
companion_added_keys_tmp=$(mktemp target/test262-default-parameters-global-companion-added-keys.XXXXXX)
combined_paths_tmp=$(mktemp target/test262-default-parameters-global-combined-paths.XXXXXX)
combined_keys_tmp=$(mktemp target/test262-default-parameters-global-combined-keys.XXXXXX)
manifest_paths "$companion_manifest" | LC_ALL=C sort -c
awk -F'\t' '
    NR == FNR { if (NF && $1 !~ /^#/) wanted[$1]=1; next }
    $1 in wanted {
        if (seen[$1]++ || $5 != "parse" || $6 != "SyntaxError") exit 2
        print
    }
    END {
        for (path in wanted) if (!(path in seen)) exit 3
    }
' "$companion_manifest" "$metadata_tsv_tmp" >"$companion_metadata_tmp"
awk -F'\t' '
    function has(list, value) {
        return index("," list ",", "," value ",") != 0
    }
    {
        if (has($3, "module") || has($3, "raw") ||
            (has($3, "onlyStrict") && has($3, "noStrict"))) exit 2
        if (has($3, "onlyStrict")) print $1 "\tstrict"
        else if (has($3, "noStrict")) print $1 "\tsloppy"
        else {
            print $1 "\tsloppy"
            print $1 "\tstrict"
        }
    }
' "$companion_metadata_tmp" | LC_ALL=C sort >"$companion_keys_tmp"
comm -12 <(manifest_paths "$companion_manifest") \
    <(printf '%s\n' "$parent_negatives") >"$companion_parent_tmp"
comm -23 <(manifest_paths "$companion_manifest") \
    <(printf '%s\n' "$parent_negatives") >"$companion_added_tmp"
awk '{ print $0 "\tsloppy"; print $0 "\tstrict" }' "$companion_added_tmp" \
    | LC_ALL=C sort >"$companion_added_keys_tmp"
LC_ALL=C sort -u "$tag_manifest" "$companion_manifest" >"$combined_paths_tmp"
LC_ALL=C sort -u "$derived_keys_tmp" "$companion_keys_tmp" >"$combined_keys_tmp"
diff -u "$companion_added_tmp" \
    <(printf '%s\n' "$companion_added_negatives")
[[ "$(read_value companion_manifest)" == "$companion_manifest" \
    && "$(wc -l <"$companion_metadata_tmp" | tr -d '[:space:]')" \
        == "$(read_value companion_paths)" \
    && "$(sha256_file "$companion_manifest")" \
        == "$(read_value companion_paths_sha256)" \
    && "$(sha256_file "$companion_metadata_tmp")" \
        == "$(read_value companion_metadata_sha256)" \
    && "$(wc -l <"$companion_keys_tmp" | tr -d '[:space:]')" \
        == "$(read_value companion_variants)" \
    && "$(sha256_file "$companion_keys_tmp")" \
        == "$(read_value companion_keys_sha256)" \
    && "$(wc -l <"$companion_parent_tmp" | tr -d '[:space:]')" \
        == "$(read_value companion_parent_audited_paths)" \
    && "$(sha256_file "$companion_parent_tmp")" \
        == "$(read_value companion_parent_audited_sha256)" \
    && "$(wc -l <"$companion_added_tmp" | tr -d '[:space:]')" \
        == "$(read_value companion_added_paths)" \
    && "$(sha256_file "$companion_added_tmp")" \
        == "$(read_value companion_added_paths_sha256)" \
    && "$(wc -l <"$companion_added_keys_tmp" | tr -d '[:space:]')" \
        == "$(read_value companion_added_variants)" \
    && "$(sha256_file "$companion_added_keys_tmp")" \
        == "$(read_value companion_added_keys_sha256)" \
    && "$(wc -l <"$combined_paths_tmp" | tr -d '[:space:]')" \
        == "$(read_value combined_paths)" \
    && "$(sha256_file "$combined_paths_tmp")" \
        == "$(read_value combined_paths_sha256)" \
    && "$(wc -l <"$combined_keys_tmp" | tr -d '[:space:]')" \
        == "$(read_value combined_variants)" \
    && "$(sha256_file "$combined_keys_tmp")" \
        == "$(read_value combined_keys_sha256)" ]] || {
    echo 'error: strict-body companion inventory drifted' >&2
    exit 1
}

# The current live profile must remain consumable by the stable runner.
live_probe=$(manifest_paths "$companion_manifest" | sed -n '1p')
live_validation_report="$run_dir/live-profile.tsv"
live_validation_json_report="$run_dir/live-profile.jsonl"
live_validation_output=$(
    "$runner" \
        --suite "$suite" \
        --config "$source_dir/test262.conf" \
        --oxide-profile "$live_profile" \
        --test "$live_probe" \
        --report "$live_validation_report" \
        --mode "$(read_value mode)" \
        --workers 1 \
        --timeout-ms "$(read_value timeout_ms)" \
        --allow-failures
)
verify_report "$live_validation_report" "$live_validation_json_report" \
    "$live_profile_sha256" 2 pass=2
[[ "$(execution_runnable "$live_validation_output")" == 2 ]] || {
    echo 'error: live profile runner probe drifted' >&2
    exit 1
}

# The global admission stays downstream of the immutable focused certificate.
"$focused_gate" --frozen-profiles
focused_sha=$(sha256_file "$focused_baseline")
[[ "$mode" == bless-tag \
    || "$focused_sha" == "$(read_value focused_baseline_sha256)" ]] || {
    echo 'error: focused default-parameters baseline identity drifted' >&2
    exit 1
}

# Authenticate the untagged negative provenance with both engines.
companion_worker_tmp=$(mktemp target/test262-default-parameters-global-companion-worker.XXXXXX)
: >"$companion_worker_tmp"
while IFS=$'\t' read -r test_path variant; do
    result=$("$runner" --worker-one --suite "$suite" \
        --test "$test_path" --variant "$variant")
    printf '%s\t%s\t%s\n' "$test_path" "$variant" "$result" \
        >>"$companion_worker_tmp"
done <"$companion_keys_tmp"
[[ "$(wc -l <"$companion_worker_tmp" | tr -d '[:space:]')" \
        == "$(read_value companion_worker_variants)" ]] \
    && awk -F'\t' '
        NF != 6 || $3 != "pass" || $4 != "parse" ||
            $5 != "SyntaxError" || $6 == "" { exit 1 }
    ' "$companion_worker_tmp" || {
    echo 'error: raw Oxide strict-body receipt drifted' >&2
    exit 1
}
[[ "$mode" == bless-tag \
    || "$(sha256_file "$companion_worker_tmp")" \
        == "$(read_value companion_worker_receipt_sha256)" ]] || {
    echo 'error: raw Oxide strict-body receipt checksum drifted' >&2
    exit 1
}
companion_quickjs_tmp=$(mktemp target/test262-default-parameters-global-companion-quickjs.XXXXXX)
quickjs_files=()
while IFS= read -r test_path; do
    quickjs_files+=("test262/$test_path")
done <"$companion_manifest"
[[ "${#quickjs_files[@]}" == "$(read_value companion_quickjs_paths)" ]] || {
    echo 'error: strict-body QuickJS path count drifted' >&2
    exit 1
}
if ! (
    cd -- "$source_dir"
    ./run-test262 -m -c test262.conf -a -T 1 -f "${quickjs_files[@]}"
) >"$companion_quickjs_tmp" 2>&1; then
    tail -n 100 "$companion_quickjs_tmp" >&2
    echo 'error: pinned QuickJS failed the strict-body companion' >&2
    exit 1
fi
if grep -Eq '(^|[[:space:]])FAILED($|[[:space:]])|SKIPPED FEATURE' \
        "$companion_quickjs_tmp" \
    || ! grep -Fq \
        "Average memory statistics for $(read_value companion_quickjs_variants) tests:" \
        "$companion_quickjs_tmp"; then
    tail -n 100 "$companion_quickjs_tmp" >&2
    echo 'error: pinned QuickJS strict-body receipt drifted' >&2
    exit 1
fi
[[ "$mode" == bless-tag \
    || "$(sha256_file "$companion_quickjs_tmp")" \
        == "$(read_value companion_quickjs_log_sha256)" ]] || {
    echo 'error: pinned QuickJS strict-body receipt checksum drifted' >&2
    exit 1
}

if [[ "$mode" == check ]]; then
    echo 'default-parameters global inputs exact: 2283 paths / 4544 variants'
    exit 0
fi

# Run the tag and companion cohorts independently; their disjoint sorted union
# is the complete global admission transition.
rm -f -- "$parent_tag_report" "$parent_tag_json_report"
parent_tag_output=$(run_tag "$parent_profile" "$parent_tag_report")
printf '%s\n' "$parent_tag_output"
verify_report "$parent_tag_report" "$parent_tag_json_report" \
    "$(read_value parent_oxide_profile_sha256)" \
    "$(read_value tag_variants)" "$(read_value parent_tag_summary)"
[[ "$(execution_runnable "$parent_tag_output")" \
        == "$(read_value parent_tag_runnable)" \
    && "$(report_keys "$parent_tag_report" | sha256_stream)" \
        == "$(read_value tag_keys_sha256)" ]] || {
    echo 'error: parent tag vector drifted' >&2
    exit 1
}

rm -f -- "$candidate_tag_report" "$candidate_tag_json_report"
candidate_tag_output=$(run_tag "$candidate_profile" "$candidate_tag_report")
printf '%s\n' "$candidate_tag_output"
verify_report "$candidate_tag_report" "$candidate_tag_json_report" \
    "$(read_value candidate_oxide_profile_sha256)" \
    "$(read_value tag_variants)" "$(read_value candidate_tag_summary)"
[[ "$(execution_runnable "$candidate_tag_output")" \
        == "$(read_value candidate_tag_runnable)" \
    && "$(report_keys "$candidate_tag_report" | sha256_stream)" \
        == "$(read_value tag_keys_sha256)" ]] || {
    echo 'error: candidate tag vector drifted' >&2
    exit 1
}

rm -f -- "$parent_companion_report" "$parent_companion_json_report"
parent_companion_output=$(run_companion "$parent_profile" \
    "$parent_companion_report")
printf '%s\n' "$parent_companion_output"
verify_report "$parent_companion_report" "$parent_companion_json_report" \
    "$(read_value parent_oxide_profile_sha256)" \
    "$(read_value companion_variants)" "$(read_value parent_companion_summary)"
[[ "$(execution_runnable "$parent_companion_output")" \
        == "$(read_value parent_companion_runnable)" \
    && "$(report_keys "$parent_companion_report" | sha256_stream)" \
        == "$(read_value companion_keys_sha256)" ]] || {
    echo 'error: parent strict-body vector drifted' >&2
    exit 1
}

rm -f -- "$candidate_companion_report" "$candidate_companion_json_report"
candidate_companion_output=$(run_companion "$candidate_profile" \
    "$candidate_companion_report")
printf '%s\n' "$candidate_companion_output"
verify_report "$candidate_companion_report" "$candidate_companion_json_report" \
    "$(read_value candidate_oxide_profile_sha256)" \
    "$(read_value companion_variants)" "$(read_value candidate_companion_summary)"
[[ "$(execution_runnable "$candidate_companion_output")" \
        == "$(read_value candidate_companion_runnable)" \
    && "$(report_keys "$candidate_companion_report" | sha256_stream)" \
        == "$(read_value companion_keys_sha256)" ]] || {
    echo 'error: candidate strict-body vector drifted' >&2
    exit 1
}

parent_combined_tmp=$(mktemp target/test262-default-parameters-global-parent-combined.XXXXXX)
candidate_combined_tmp=$(mktemp target/test262-default-parameters-global-candidate-combined.XXXXXX)
parent_combined_json_tmp=$(mktemp target/test262-default-parameters-global-parent-combined-json.XXXXXX)
candidate_combined_json_tmp=$(mktemp target/test262-default-parameters-global-candidate-combined-json.XXXXXX)
{
    report_rows "$parent_tag_report"
    report_rows "$parent_companion_report"
} | LC_ALL=C sort >"$parent_combined_tmp"
{
    report_rows "$candidate_tag_report"
    report_rows "$candidate_companion_report"
} | LC_ALL=C sort >"$candidate_combined_tmp"
{
    awk '/^\{"kind":"result"/' "$parent_tag_json_report"
    awk '/^\{"kind":"result"/' "$parent_companion_json_report"
} | LC_ALL=C sort >"$parent_combined_json_tmp"
{
    awk '/^\{"kind":"result"/' "$candidate_tag_json_report"
    awk '/^\{"kind":"result"/' "$candidate_companion_json_report"
} | LC_ALL=C sort >"$candidate_combined_json_tmp"
parent_combined_tsv_data=$(sha256_file "$parent_combined_tmp")
parent_combined_jsonl_data=$(sha256_file "$parent_combined_json_tmp")
candidate_combined_tsv_data=$(sha256_file "$candidate_combined_tmp")
candidate_combined_jsonl_data=$(sha256_file "$candidate_combined_json_tmp")
[[ "$(wc -l <"$parent_combined_tmp" | tr -d '[:space:]')" \
        == "$(read_value combined_variants)" \
    && "$(wc -l <"$candidate_combined_tmp" | tr -d '[:space:]')" \
        == "$(read_value combined_variants)" \
    && "$(( $(read_value parent_tag_runnable) + $(read_value parent_companion_runnable) ))" \
        == "$(read_value parent_combined_runnable)" \
    && "$(( $(read_value candidate_tag_runnable) + $(read_value candidate_companion_runnable) ))" \
        == "$(read_value candidate_combined_runnable)" ]] || {
    echo 'error: combined scoped vector drifted' >&2
    exit 1
}
if [[ "$mode" != bless-tag ]]; then
    [[ "$parent_combined_tsv_data" \
            == "$(read_value parent_combined_tsv_data_sha256)" \
        && "$parent_combined_jsonl_data" \
            == "$(read_value parent_combined_jsonl_data_sha256)" \
        && "$candidate_combined_tsv_data" \
            == "$(read_value candidate_combined_tsv_data_sha256)" \
        && "$candidate_combined_jsonl_data" \
            == "$(read_value candidate_combined_jsonl_data_sha256)" ]] || {
        echo 'error: combined scoped receipt checksum drifted' >&2
        exit 1
    }
fi

transition_tmp=$(mktemp "$transition_receipt.XXXXXX")
{
    printf '# Exhaustive pinned Test262 default-parameters global admission transition.\n'
    printf '# before_oxide_profile_sha256=%s\n' \
        "$(read_value parent_oxide_profile_sha256)"
    printf '# after_oxide_profile_sha256=%s\n' \
        "$(read_value candidate_oxide_profile_sha256)"
    printf '# manifest_sha256=%s\n' "$(read_value combined_paths_sha256)"
    printf 'path\tvariant\tflags\tfeatures\texpected_phase\texpected_type\tbefore_outcome\tbefore_actual_phase\tbefore_actual_type\tbefore_detail\tafter_outcome\tafter_actual_phase\tafter_actual_type\tafter_detail\n'
    awk -F'\t' -v OFS='\t' '
        NR == FNR {
            key=$1 SUBSEP $2
            if (key in before) exit 2
            for (i=1; i<=10; i++) old[key, i]=$i
            before[key]=1
            next
        }
        {
            key=$1 SUBSEP $2
            if (!(key in before) || key in after) exit 3
            for (i=1; i<=6; i++) if ($i != old[key, i]) exit 4
            print $1, $2, $3, $4, $5, $6,
                old[key, 7], old[key, 8], old[key, 9], old[key, 10],
                $7, $8, $9, $10
            after[key]=1
        }
        END { for (key in before) if (!(key in after)) exit 5 }
    ' "$parent_combined_tmp" "$candidate_combined_tmp"
} >"$transition_tmp"
transition_counts=$(awk -F'\t' '
    /^#/ || ($1 == "path" && $2 == "variant") { next }
    {
        rows++
        changed=0
        for (i=7; i<=10; i++) if ($i != $(i+4)) changed=1
        if (!changed) { unchanged++; next }
        changes++
        if ($7 != $11) outcome_changes++
        else detail_only++
        if ($7 == "unsupported-feature" && $11 == "pass") uf_pass++
        else if ($7 == "unsupported-feature" &&
            $11 == "unsupported-feature") uf_detail++
        else if ($7 == "unsupported-negative-provenance" && $11 == "pass") {
            unp_pass++
        } else invalid++
    }
    END {
        print rows+0, changes+0, outcome_changes+0, detail_only+0,
            unchanged+0, uf_pass+0, uf_detail+0, unp_pass+0, invalid+0
    }
' "$transition_tmp")
read -r transition_rows transition_changes transition_outcomes \
    transition_details transition_unchanged uf_pass uf_detail unp_pass \
    transition_invalid <<<"$transition_counts"
[[ "$transition_rows" == "$(read_value transition_rows)" \
    && "$transition_changes" == "$(read_value transition_changed_rows)" \
    && "$transition_outcomes" == "$(read_value transition_outcome_changed_rows)" \
    && "$transition_details" == "$(read_value transition_detail_only_rows)" \
    && "$transition_unchanged" == "$(read_value transition_unchanged_rows)" \
    && "$uf_pass" == 3352 && "$uf_detail" == 1162 && "$unp_pass" == 22 \
    && "$transition_invalid" == 0 ]] || {
    echo 'error: exact combined transition matrix drifted' >&2
    exit 1
}

transition_sha=$(sha256_file "$transition_tmp")
transition_data_sha=$(awk '!/^#/ && !/^path\tvariant\t/' "$transition_tmp" \
    | sha256_stream)
parent_tag_nonpass=$(report_nonpass_sha256 "$parent_tag_report")
candidate_tag_nonpass=$(report_nonpass_sha256 "$candidate_tag_report")
parent_companion_nonpass=$(report_nonpass_sha256 "$parent_companion_report")
candidate_companion_nonpass=$(report_nonpass_sha256 "$candidate_companion_report")
parent_tag_tsv=$(sha256_file "$parent_tag_report")
parent_tag_jsonl=$(sha256_file "$parent_tag_json_report")
candidate_tag_tsv=$(sha256_file "$candidate_tag_report")
candidate_tag_jsonl=$(sha256_file "$candidate_tag_json_report")
parent_companion_tsv=$(sha256_file "$parent_companion_report")
parent_companion_jsonl=$(sha256_file "$parent_companion_json_report")
candidate_companion_tsv=$(sha256_file "$candidate_companion_report")
candidate_companion_jsonl=$(sha256_file "$candidate_companion_json_report")

if [[ "$mode" == bless-tag ]]; then
    chmod 644 "$transition_tmp"
    mv -- "$transition_tmp" "$transition_receipt"
    transition_tmp=
    update_values "$baseline" \
        "focused_baseline_sha256=$focused_sha" \
        "companion_worker_receipt_sha256=$(sha256_file "$companion_worker_tmp")" \
        "companion_quickjs_log_sha256=$(sha256_file "$companion_quickjs_tmp")" \
        "transition_receipt_sha256=$transition_sha" \
        "transition_data_sha256=$transition_data_sha" \
        "parent_tag_nonpass_sha256=$parent_tag_nonpass" \
        "parent_tag_tsv_sha256=$parent_tag_tsv" \
        "parent_tag_jsonl_sha256=$parent_tag_jsonl" \
        "candidate_tag_nonpass_sha256=$candidate_tag_nonpass" \
        "candidate_tag_tsv_sha256=$candidate_tag_tsv" \
        "candidate_tag_jsonl_sha256=$candidate_tag_jsonl" \
        "parent_companion_nonpass_sha256=$parent_companion_nonpass" \
        "parent_companion_tsv_sha256=$parent_companion_tsv" \
        "parent_companion_jsonl_sha256=$parent_companion_jsonl" \
        "candidate_companion_nonpass_sha256=$candidate_companion_nonpass" \
        "candidate_companion_tsv_sha256=$candidate_companion_tsv" \
        "candidate_companion_jsonl_sha256=$candidate_companion_jsonl" \
        "parent_combined_tsv_data_sha256=$parent_combined_tsv_data" \
        "parent_combined_jsonl_data_sha256=$parent_combined_jsonl_data" \
        "candidate_combined_tsv_data_sha256=$candidate_combined_tsv_data" \
        "candidate_combined_jsonl_data_sha256=$candidate_combined_jsonl_data"
    echo 'default-parameters 4544-row global transition blessed'
    exit 0
fi

for entry in \
    "transition_receipt_sha256:$transition_sha" \
    "transition_data_sha256:$transition_data_sha" \
    "parent_tag_nonpass_sha256:$parent_tag_nonpass" \
    "parent_tag_tsv_sha256:$parent_tag_tsv" \
    "parent_tag_jsonl_sha256:$parent_tag_jsonl" \
    "candidate_tag_nonpass_sha256:$candidate_tag_nonpass" \
    "candidate_tag_tsv_sha256:$candidate_tag_tsv" \
    "candidate_tag_jsonl_sha256:$candidate_tag_jsonl" \
    "parent_companion_nonpass_sha256:$parent_companion_nonpass" \
    "parent_companion_tsv_sha256:$parent_companion_tsv" \
    "parent_companion_jsonl_sha256:$parent_companion_jsonl" \
    "candidate_companion_nonpass_sha256:$candidate_companion_nonpass" \
    "candidate_companion_tsv_sha256:$candidate_companion_tsv" \
    "candidate_companion_jsonl_sha256:$candidate_companion_jsonl"; do
    key=${entry%%:*}
    actual=${entry#*:}
    [[ "$actual" == "$(read_value "$key")" ]] || {
        printf 'error: scoped receipt drifted: %s\n' "$key" >&2
        exit 1
    }
done
cmp -s "$transition_tmp" "$transition_receipt" || {
    echo 'error: checked-in transition receipt drifted' >&2
    exit 1
}
rm -f -- "$transition_tmp"
transition_tmp=

if [[ "$mode" == tag ]]; then
    echo 'default-parameters global transition exact: 3374 outcomes, 1162 detail-only, 8 unchanged'
    exit 0
fi

# --full always executes fresh parent and candidate receipts and proves the
# complete 102,037-key join; no projected report is accepted here.
rm -f -- "$parent_full_report" "$parent_full_json_report"
parent_full_output=$(run_full "$parent_profile" "$parent_full_report")
printf '%s\n' "$parent_full_output"
verify_report "$parent_full_report" "$parent_full_json_report" \
    "$(read_value parent_oxide_profile_sha256)" "$(read_value full_variants)" \
    "$(read_value parent_full_summary)"
[[ "$(execution_runnable "$parent_full_output")" \
        == "$(read_value parent_full_runnable)" \
    && "$(report_outcome_count "$parent_full_report" pass)" \
        == "$(read_value parent_full_passes)" \
    && "$(report_outcome_count "$parent_full_report" unsupported-feature)" \
        == "$(read_value parent_full_unsupported_feature)" \
    && "$(report_outcome_count "$parent_full_report" unsupported-negative-provenance)" \
        == "$(read_value parent_full_unsupported_negative_provenance)" \
    && "$(unsupported_total "$(report_summary "$parent_full_report")")" \
        == "$(read_value parent_full_total_unsupported)" \
    && "$(sha256_file "$parent_full_report")" \
        == "$(read_value parent_full_tsv_sha256)" \
    && "$(sha256_file "$parent_full_json_report")" \
        == "$(read_value parent_full_jsonl_sha256)" \
    && "$(report_keys "$parent_full_report" | sha256_stream)" \
        == "$(read_value full_keys_sha256)" ]] || {
    echo 'error: authoritative parent full receipt drifted' >&2
    exit 1
}

rm -f -- "$candidate_full_report" "$candidate_full_json_report"
candidate_full_output=$(run_full "$candidate_profile" "$candidate_full_report")
printf '%s\n' "$candidate_full_output"
verify_report "$candidate_full_report" "$candidate_full_json_report" \
    "$(read_value candidate_oxide_profile_sha256)" "$(read_value full_variants)" \
    "$(read_value candidate_full_summary)"
[[ "$(execution_runnable "$candidate_full_output")" \
        == "$(read_value candidate_full_runnable)" \
    && "$(report_outcome_count "$candidate_full_report" pass)" \
        == "$(read_value candidate_full_passes)" \
    && "$(report_outcome_count "$candidate_full_report" unsupported-feature)" \
        == "$(read_value candidate_full_unsupported_feature)" \
    && "$(report_outcome_count "$candidate_full_report" unsupported-negative-provenance)" \
        == "$(read_value candidate_full_unsupported_negative_provenance)" \
    && "$(unsupported_total "$(report_summary "$candidate_full_report")")" \
        == "$(read_value candidate_full_total_unsupported)" \
    && "$(sha256_file "$candidate_full_report")" \
        == "$(read_value candidate_full_tsv_sha256)" \
    && "$(sha256_file "$candidate_full_json_report")" \
        == "$(read_value candidate_full_jsonl_sha256)" \
    && "$(report_keys "$candidate_full_report" | sha256_stream)" \
        == "$(read_value full_keys_sha256)" ]] || {
    echo 'error: candidate full receipt drifted' >&2
    exit 1
}
diff -u <(report_keys "$parent_full_report") \
    <(report_keys "$candidate_full_report")

parent_full_combined=$(rows_for_paths "$combined_paths_tmp" "$parent_full_report")
candidate_full_combined=$(rows_for_paths "$combined_paths_tmp" "$candidate_full_report")
parent_noncombined=$(rows_without_paths "$combined_paths_tmp" "$parent_full_report")
candidate_noncombined=$(rows_without_paths "$combined_paths_tmp" "$candidate_full_report")
parent_full_combined_json=$(json_rows_for_paths "$combined_paths_tmp" \
    "$parent_full_json_report")
candidate_full_combined_json=$(json_rows_for_paths "$combined_paths_tmp" \
    "$candidate_full_json_report")
parent_noncombined_json=$(json_rows_without_paths "$combined_paths_tmp" \
    "$parent_full_json_report")
candidate_noncombined_json=$(json_rows_without_paths "$combined_paths_tmp" \
    "$candidate_full_json_report")
[[ "$(printf '%s\n' "$parent_full_combined" | wc -l | tr -d '[:space:]')" \
        == "$(read_value full_combined_rows)" \
    && "$(printf '%s\n' "$candidate_full_combined" | wc -l | tr -d '[:space:]')" \
        == "$(read_value full_combined_rows)" \
    && "$(printf '%s\n' "$parent_noncombined" | wc -l | tr -d '[:space:]')" \
        == "$(read_value full_noncombined_rows)" \
    && "$(printf '%s\n' "$candidate_noncombined" | wc -l | tr -d '[:space:]')" \
        == "$(read_value full_noncombined_rows)" ]] || {
    echo 'error: full combined/complement partition drifted' >&2
    exit 1
}
diff -u "$parent_combined_tmp" <(printf '%s\n' "$parent_full_combined")
diff -u "$candidate_combined_tmp" <(printf '%s\n' "$candidate_full_combined")
diff -u "$parent_combined_json_tmp" \
    <(printf '%s\n' "$parent_full_combined_json")
diff -u "$candidate_combined_json_tmp" \
    <(printf '%s\n' "$candidate_full_combined_json")
diff -u <(printf '%s\n' "$parent_noncombined") \
    <(printf '%s\n' "$candidate_noncombined")
diff -u <(printf '%s\n' "$parent_noncombined_json") \
    <(printf '%s\n' "$candidate_noncombined_json")

changed_keys_tmp=$(mktemp target/test262-default-parameters-global-changed.XXXXXX)
join_counts=$(awk -F'\t' -v changed_file="$changed_keys_tmp" '
    NR == FNR {
        if (/^#/ || ($1 == "path" && $2 == "variant")) next
        key=$1 SUBSEP $2
        if (key in before) exit 2
        for (i=1; i<=10; i++) old[key, i]=$i
        before[key]=1
        before_count++
        next
    }
    /^#/ || ($1 == "path" && $2 == "variant") { next }
    {
        key=$1 SUBSEP $2
        if (!(key in before) || key in after) exit 3
        for (i=1; i<=6; i++) if ($i != old[key, i]) exit 4
        if (old[key, 7] == "pass" && $7 != "pass") regressions++
        changed=0
        for (i=7; i<=10; i++) if ($i != old[key, i]) changed=1
        if (changed) {
            changes++
            print $1 "\t" $2 >changed_file
            if (old[key, 7] != $7) outcome_changes++
            else detail_only++
            if (old[key, 7] == "unsupported-feature" && $7 == "pass") {
                uf_pass++
            } else if (old[key, 7] == "unsupported-feature" &&
                $7 == "unsupported-feature") {
                uf_detail++
            } else if (old[key, 7] == "unsupported-negative-provenance" &&
                $7 == "pass") {
                unp_pass++
            } else invalid++
        } else unchanged++
        after[key]=1
        after_count++
    }
    END {
        if (before_count != after_count) exit 5
        for (key in before) if (!(key in after)) exit 6
        print before_count+0, changes+0, outcome_changes+0, detail_only+0,
            unchanged+0, regressions+0, uf_pass+0, uf_detail+0,
            unp_pass+0, invalid+0
    }
' "$parent_full_report" "$candidate_full_report") || {
    echo 'error: complete full keyed join failed' >&2
    exit 1
}
read -r full_rows changed_rows outcome_changed_rows detail_only_rows \
    unchanged_rows regressions full_uf_pass full_uf_detail full_unp_pass \
    invalid_changes <<<"$join_counts"
[[ "$full_rows" == "$(read_value full_variants)" \
    && "$changed_rows" == "$(read_value full_changed_rows)" \
    && "$outcome_changed_rows" == "$(read_value full_outcome_changed_rows)" \
    && "$detail_only_rows" == "$(read_value full_detail_only_rows)" \
    && "$unchanged_rows" == "$(read_value full_unchanged_rows)" \
    && "$regressions" == "$(read_value previous_pass_regressions)" \
    && "$full_uf_pass" == 3352 && "$full_uf_detail" == 1162 \
    && "$full_unp_pass" == 22 && "$invalid_changes" == 0 ]] || {
    echo 'error: full keyed join matrix drifted' >&2
    exit 1
}
expected_changed_keys=$(awk -F'\t' '
    /^#/ || ($1 == "path" && $2 == "variant") { next }
    {
        for (i=7; i<=10; i++) if ($i != $(i+4)) {
            print $1 "\t" $2
            next
        }
    }
' "$transition_receipt")
diff -u <(printf '%s\n' "$expected_changed_keys") "$changed_keys_tmp"

parent_combined_tsv_sha=$(printf '%s\n' "$parent_full_combined" | sha256_stream)
candidate_combined_tsv_sha=$(printf '%s\n' "$candidate_full_combined" | sha256_stream)
parent_combined_json_sha=$(printf '%s\n' "$parent_full_combined_json" | sha256_stream)
candidate_combined_json_sha=$(printf '%s\n' "$candidate_full_combined_json" | sha256_stream)
noncombined_tsv_sha=$(printf '%s\n' "$parent_noncombined" | sha256_stream)
noncombined_json_sha=$(printf '%s\n' "$parent_noncombined_json" | sha256_stream)
for entry in \
    "full_parent_combined_tsv_data_sha256:$parent_combined_tsv_sha" \
    "full_parent_combined_jsonl_data_sha256:$parent_combined_json_sha" \
    "full_candidate_combined_tsv_data_sha256:$candidate_combined_tsv_sha" \
    "full_candidate_combined_jsonl_data_sha256:$candidate_combined_json_sha" \
    "full_noncombined_tsv_data_sha256:$noncombined_tsv_sha" \
    "full_noncombined_jsonl_data_sha256:$noncombined_json_sha"; do
    key=${entry%%:*}
    actual=${entry#*:}
    [[ "$actual" == "$(read_value "$key")" ]] || {
        printf 'error: full partition receipt drifted: %s\n' "$key" >&2
        exit 1
    }
done

echo 'default-parameters full transition exact: 3374 outcomes, 1162 detail-only, 97493-byte-identical complement, zero regressions'
