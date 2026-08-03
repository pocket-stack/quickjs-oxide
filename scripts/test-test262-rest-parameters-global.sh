#!/usr/bin/env bash
# Reproduce the exact global Test262 admission for rest-parameters.

set -euo pipefail
export TZ=America/Los_Angeles
export LC_ALL=C

script_dir=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd)
root=$(CDPATH='' cd -- "$script_dir/.." && pwd)
baseline=tests/test262-rest-parameters-global-baseline.txt
canonical_baseline=tests/test262-full-baseline.txt
focused_baseline=tests/test262-rest-parameters-baseline.txt
focused_gate=scripts/test-test262-rest-parameters.sh
parent_profile=tests/test262-rest-parameters-parent.conf
candidate_profile=tests/test262-rest-parameters-candidate.conf
live_profile=compat/test262-oxide.conf
universe_manifest=tests/test262-rest-parameters-universe.txt
activation_manifest=tests/test262-rest-parameters-activation.txt
transition_receipt=tests/test262-rest-parameters-global-transitions.tsv
parent_report=target/test262-rest-parameters-global-parent.tsv
parent_json_report=target/test262-rest-parameters-global-parent.jsonl
candidate_report=target/test262-rest-parameters-global-candidate.tsv
candidate_json_report=target/test262-rest-parameters-global-candidate.jsonl
parent_full_report=target/test262-rest-parameters-global-parent-full.tsv
parent_full_json_report=target/test262-rest-parameters-global-parent-full.jsonl
candidate_full_report=target/test262-rest-parameters-global-candidate-full.tsv
candidate_full_json_report=target/test262-rest-parameters-global-candidate-full.jsonl
workers=${TEST262_WORKERS:-8}
full_workers=${TEST262_FULL_WORKERS:-2}
lock_dir="$root/target/test262-rest-parameters-global.lock"
lock_held=0
run_dir=
runner=
metadata_tmp=
metadata_tsv_tmp=
derived_paths_tmp=
derived_keys_tmp=
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
    printf '  --bless-full  intentionally replace full candidate/join hashes\n'
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
        execution_entries execution_sha256 universe_paths universe_sha256 \
        universe_variants universe_keys_sha256 activation_paths activation_sha256 \
        activation_variants transition_receipt transition_rows \
        transition_receipt_sha256 transition_data_sha256 parent_tag_runnable \
        parent_tag_summary parent_tag_nonpass_sha256 parent_tag_tsv_sha256 \
        parent_tag_jsonl_sha256 candidate_tag_runnable candidate_tag_summary \
        candidate_tag_nonpass_sha256 candidate_tag_tsv_sha256 \
        candidate_tag_jsonl_sha256 full_variants full_keys_sha256 \
        full_universe_rows full_non_universe_rows full_changed_rows \
        full_outcome_changed_rows full_detail_only_rows full_unchanged_rows \
        previous_pass_regressions parent_full_runnable parent_full_passes \
        parent_full_unsupported_feature parent_full_total_unsupported \
        parent_full_summary parent_full_tsv_sha256 parent_full_jsonl_sha256 \
        candidate_full_runnable candidate_full_passes \
        candidate_full_unsupported_feature candidate_full_total_unsupported \
        candidate_full_summary candidate_full_tsv_sha256 \
        candidate_full_jsonl_sha256 full_parent_universe_tsv_data_sha256 \
        full_parent_universe_jsonl_data_sha256 \
        full_candidate_universe_tsv_data_sha256 \
        full_candidate_universe_jsonl_data_sha256 \
        full_non_universe_tsv_data_sha256 full_non_universe_jsonl_data_sha256
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
        printf 'error: rest-parameters global baseline drifted for %s: %s != %s\n' \
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
        --manifest "$universe_manifest" \
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
    printf 'error: rest-parameters global gate is already running: %s\n' \
        "$lock_dir" >&2
    exit 1
fi
lock_held=1
printf '%s\n' "$$" >"$lock_dir/pid"
validate_baseline_schema
run_dir=$(mktemp -d target/test262-rest-parameters-global-run.XXXXXX)
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
expect_value focused_baseline tests/test262-rest-parameters-baseline.txt
expect_value parent_oxide_profile_sha256 fc2716ff2ef12fda73c33db0603525f100713ff3b6df0ac8205977a20717ea3a
expect_value candidate_oxide_profile_sha256 d55e0625b1f6878b7afa6885d82cf332909271ce1c2222100fe3a403a8455969
expect_value parent_features 91
expect_value candidate_features 92
expect_value added_features 1
expect_value parent_audited_negative_tests 828
expect_value candidate_audited_negative_tests 924
expect_value added_audited_negative_tests 96
expect_value execution_entries 1
expect_value universe_paths 96
expect_value universe_variants 192
expect_value activation_paths 96
expect_value activation_variants 192
expect_value transition_receipt tests/test262-rest-parameters-global-transitions.tsv
expect_value transition_rows 192
expect_value parent_tag_runnable 0
expect_value parent_tag_summary unsupported-feature=192
expect_value candidate_tag_runnable 192
expect_value candidate_tag_summary pass=192
expect_value full_variants 102037
expect_value full_universe_rows 192
expect_value full_non_universe_rows 101845
expect_value full_changed_rows 192
expect_value full_outcome_changed_rows 192
expect_value full_detail_only_rows 0
expect_value full_unchanged_rows 101845
expect_value previous_pass_regressions 0
expect_value parent_full_runnable 60026
expect_value parent_full_passes 59507
expect_value parent_full_unsupported_feature 18618
expect_value parent_full_total_unsupported 23585
expect_value candidate_full_runnable 60218
expect_value candidate_full_passes 59699
expect_value candidate_full_unsupported_feature 18426
expect_value candidate_full_total_unsupported 23393

full_receipt_fields=(
    candidate_full_tsv_sha256
    candidate_full_jsonl_sha256
    full_parent_universe_tsv_data_sha256
    full_parent_universe_jsonl_data_sha256
    full_candidate_universe_tsv_data_sha256
    full_candidate_universe_jsonl_data_sha256
    full_non_universe_tsv_data_sha256
    full_non_universe_jsonl_data_sha256
)
full_pending_count=0
for key in "${full_receipt_fields[@]}"; do
    [[ "$(read_value "$key")" != PENDING ]] || \
        full_pending_count=$((full_pending_count + 1))
done
if [[ "$full_pending_count" != 0 \
    && "$full_pending_count" != "${#full_receipt_fields[@]}" ]]; then
    echo 'error: full receipt is only partially PENDING' >&2
    exit 1
fi
full_bootstrap=0
if [[ "$full_pending_count" == "${#full_receipt_fields[@]}" ]]; then
    full_bootstrap=1
    if [[ "$mode" == full ]]; then
        echo 'error: full receipt needs the explicit --bless-full bootstrap' >&2
        exit 1
    fi
elif [[ "$mode" == bless-full ]]; then
    # A finalized receipt is immutable. The blessing spelling becomes the
    # ordinary strict verification mode instead of overwriting frozen hashes.
    mode=full
fi

for required in "$baseline" "$canonical_baseline" "$parent_profile" \
    "$candidate_profile" "$live_profile" "$universe_manifest" \
    "$activation_manifest"; do
    [[ -f "$required" ]] || {
        printf 'error: missing rest-parameters global asset: %s\n' "$required" >&2
        exit 1
    }
done
validate_key_value_schema "$canonical_baseline" expected_canonical_baseline_keys
[[ -x "$focused_gate" && -f "$focused_baseline" ]] || {
    echo 'error: focused rest-parameters gate is not ready' >&2
    exit 1
}

# Metadata is invariant across canonical admission states. During the initial
# all-PENDING --bless-full bootstrap, the canonical receipt may still be the
# exact parent or may already be the candidate. While this candidate remains
# live, every finalized mode requires its exact canonical vector. After a later
# live-profile promotion, this historical gate keeps replaying its immutable
# parent/candidate receipts; the current canonical vector belongs to the
# independent canonical full gate.
for binding in schema:schema timeout_ms:timeout_ms full_variants:variants; do
    global_key=${binding%%:*}
    canonical_key=${binding#*:}
    [[ "$(read_value "$global_key")" \
            == "$(read_value_from "$canonical_baseline" "$canonical_key")" ]] || {
        printf 'error: canonical metadata binding drifted: %s\n' "$global_key" >&2
        exit 1
    }
done

canonical_vector_matches() {
    local state=$1 runnable passes tsv jsonl summary
    if [[ "$state" == parent ]]; then
        runnable=$(read_value parent_full_runnable)
        passes=$(read_value parent_full_passes)
        tsv=$(read_value parent_full_tsv_sha256)
        jsonl=$(read_value parent_full_jsonl_sha256)
        summary=$(read_value parent_full_summary)
    else
        runnable=$(read_value candidate_full_runnable)
        passes=$(read_value candidate_full_passes)
        tsv=$(read_value candidate_full_tsv_sha256)
        jsonl=$(read_value candidate_full_jsonl_sha256)
        summary=$(read_value candidate_full_summary)
    fi
    [[ "$(read_value_from "$canonical_baseline" runnable)" == "$runnable" \
        && "$(read_value_from "$canonical_baseline" passes)" == "$passes" \
        && "$(read_value_from "$canonical_baseline" summary)" == "$summary" ]] \
        || return 1
    if [[ "$state" == candidate && "$full_bootstrap" == 1 ]]; then
        return 0
    fi
    [[ "$(read_value_from "$canonical_baseline" tsv_sha256)" == "$tsv" \
        && "$(read_value_from "$canonical_baseline" jsonl_sha256)" == "$jsonl" ]]
}

if [[ "$full_bootstrap" == 1 ]]; then
    if canonical_vector_matches parent; then
        canonical_state=parent
    elif canonical_vector_matches candidate; then
        canonical_state=candidate
    else
        echo 'error: bootstrap canonical matches neither parent nor candidate' >&2
        exit 1
    fi
else
    canonical_state=unresolved
fi

# Authenticate the exact one-feature plus 96-negative profile delta.
live_profile_sha256_before=$(sha256_file "$live_profile")
parent_features=$(profile_section "$parent_profile" features)
candidate_features=$(profile_section "$candidate_profile" features)
parent_negatives=$(profile_section "$parent_profile" audited-negative-tests)
candidate_negatives=$(profile_section "$candidate_profile" audited-negative-tests)
parent_execution=$(profile_section "$parent_profile" execution)
candidate_execution=$(profile_section "$candidate_profile" execution)
live_features=$(profile_section "$live_profile" features)
live_negatives=$(profile_section "$live_profile" audited-negative-tests)
live_execution=$(profile_section "$live_profile" execution)
if ! printf '%s\n' "$live_features" | LC_ALL=C sort -cu \
    || ! printf '%s\n' "$live_negatives" | LC_ALL=C sort -cu; then
    echo 'error: live profile entries are duplicate or out of order' >&2
    exit 1
fi
added_features=$(comm -13 <(printf '%s\n' "$parent_features") \
    <(printf '%s\n' "$candidate_features"))
removed_features=$(comm -23 <(printf '%s\n' "$parent_features") \
    <(printf '%s\n' "$candidate_features"))
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
    && "$(printf '%s\n' "$candidate_features" | wc -l | tr -d '[:space:]')" \
        == "$(read_value candidate_features)" \
    && "$(printf '%s\n' "$candidate_features" | sha256_stream)" \
        == "$(read_value candidate_features_sha256)" \
    && "$added_features" == rest-parameters \
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
    && -z "$removed_negatives" \
    && "$parent_execution" == async=true \
    && "$candidate_execution" == "$parent_execution" \
    && "$(printf '%s\n' "$candidate_execution" | wc -l | tr -d '[:space:]')" \
        == "$(read_value execution_entries)" \
    && "$(printf '%s\n' "$candidate_execution" | sha256_stream)" \
        == "$(read_value execution_sha256)" ]] || {
    echo 'error: rest-parameters profile delta drifted' >&2
    exit 1
}
diff -u <(manifest_paths "$universe_manifest") \
    <(printf '%s\n' "$added_negatives")
[[ -z "$(comm -23 <(printf '%s\n' "$candidate_features") \
        <(printf '%s\n' "$live_features"))" \
    && -z "$(comm -23 <(printf '%s\n' "$candidate_negatives") \
        <(printf '%s\n' "$live_negatives"))" ]] || {
    echo 'error: live profile removed a rest-parameters candidate capability' >&2
    exit 1
}
diff -u <(printf '%s\n' "$candidate_execution") \
    <(printf '%s\n' "$live_execution")
live_added_features=$(comm -13 <(printf '%s\n' "$candidate_features") \
    <(printf '%s\n' "$live_features"))
live_added_negatives=$(comm -13 <(printf '%s\n' "$candidate_negatives") \
    <(printf '%s\n' "$live_negatives"))
live_profile_sha256=$(sha256_file "$live_profile")
[[ "$live_profile_sha256" == "$live_profile_sha256_before" ]] || {
    echo 'error: live profile changed while the gate was authenticating it' >&2
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
if [[ "$full_bootstrap" != 1 ]]; then
    if [[ "$live_profile_sha256" \
            == "$(read_value candidate_oxide_profile_sha256)" ]]; then
        if canonical_vector_matches candidate; then
            canonical_state=candidate
        else
            echo 'error: live rest-parameters candidate requires its exact canonical vector' >&2
            exit 1
        fi
    elif [[ -n "$live_added_features" || -n "$live_added_negatives" ]]; then
        canonical_state=descendant
    else
        echo 'error: live profile differs without advancing candidate capabilities' >&2
        exit 1
    fi
fi

# Rebuild the complete tag universe and its strict/sloppy key expansion from
# the pinned 53,125-record metadata stream.
metadata_tmp=$(mktemp target/test262-rest-parameters-global-metadata.XXXXXX)
metadata_tsv_tmp=$(mktemp target/test262-rest-parameters-global-metadata-tsv.XXXXXX)
derived_paths_tmp=$(mktemp target/test262-rest-parameters-global-paths.XXXXXX)
derived_keys_tmp=$(mktemp target/test262-rest-parameters-global-keys.XXXXXX)
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
awk -F'\t' '
    {
        count=split($4, feature, ",")
        hit=0
        for (i=1; i<=count; i++) if (feature[i] == "rest-parameters") hit=1
        if (!hit) next
        if ($5 != "parse" || $6 != "SyntaxError") exit 2
        print $1
    }
' "$metadata_tsv_tmp" | LC_ALL=C sort >"$derived_paths_tmp"
awk -F'\t' '
    function has_flag(flags, wanted, count, value, i) {
        count=split(flags, value, ",")
        for (i=1; i<=count; i++) if (value[i] == wanted) return 1
        return 0
    }
    {
        count=split($4, feature, ",")
        hit=0
        for (i=1; i<=count; i++) if (feature[i] == "rest-parameters") hit=1
        if (!hit) next
        if (has_flag($3, "module") || has_flag($3, "raw") ||
            has_flag($3, "noStrict")) {
            print $1 "\tsloppy"
        } else if (has_flag($3, "onlyStrict")) {
            print $1 "\tstrict"
        } else {
            print $1 "\tsloppy"
            print $1 "\tstrict"
        }
    }
' "$metadata_tsv_tmp" | LC_ALL=C sort >"$derived_keys_tmp"
manifest_paths "$universe_manifest" | LC_ALL=C sort -c
manifest_paths "$activation_manifest" | LC_ALL=C sort -c
diff -u <(manifest_paths "$universe_manifest") "$derived_paths_tmp"
diff -u <(manifest_paths "$universe_manifest") \
    <(manifest_paths "$activation_manifest")
[[ "$(wc -l <"$derived_paths_tmp" | tr -d '[:space:]')" \
        == "$(read_value universe_paths)" \
    && "$(sha256_file "$derived_paths_tmp")" == "$(read_value universe_sha256)" \
    && "$(wc -l <"$derived_keys_tmp" | tr -d '[:space:]')" \
        == "$(read_value universe_variants)" \
    && "$(sha256_file "$derived_keys_tmp")" \
        == "$(read_value universe_keys_sha256)" \
    && "$(manifest_paths "$activation_manifest" | sha256_stream)" \
        == "$(read_value activation_sha256)" ]] || {
    echo 'error: rest-parameters metadata universe drifted' >&2
    exit 1
}

# The current runner must authenticate and parse the current live profile,
# including any future strict semantic descendant. A two-variant probe keeps
# this historical gate independent from the mutable canonical full vector.
live_probe=$(manifest_paths "$activation_manifest" | sed -n '1p')
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

# The global gate is downstream of the focused QuickJS/Oxide proof.
"$focused_gate" --frozen-profiles
focused_sha=$(sha256_file "$focused_baseline")
if [[ "$mode" != bless-tag \
    && "$focused_sha" != "$(read_value focused_baseline_sha256)" ]]; then
    echo 'error: focused rest-parameters baseline identity drifted' >&2
    exit 1
fi
if [[ "$mode" == check ]]; then
    printf 'rest-parameters inputs exact: 96 paths, 192 parse-negative variants\n'
    exit 0
fi

# Run and authenticate the complete tag transition.
rm -f -- "$parent_report" "$parent_json_report"
parent_output=$(run_tag "$parent_profile" "$parent_report")
printf '%s\n' "$parent_output"
verify_report "$parent_report" "$parent_json_report" \
    "$(read_value parent_oxide_profile_sha256)" \
    "$(read_value universe_variants)" "$(read_value parent_tag_summary)"
[[ "$(execution_runnable "$parent_output")" == "$(read_value parent_tag_runnable)" \
    && "$(report_keys "$parent_report" | sha256_stream)" \
        == "$(read_value universe_keys_sha256)" ]] || {
    echo 'error: parent tag vector drifted' >&2
    exit 1
}

rm -f -- "$candidate_report" "$candidate_json_report"
candidate_output=$(run_tag "$candidate_profile" "$candidate_report")
printf '%s\n' "$candidate_output"
verify_report "$candidate_report" "$candidate_json_report" \
    "$(read_value candidate_oxide_profile_sha256)" \
    "$(read_value universe_variants)" "$(read_value candidate_tag_summary)"
diff -u <(report_keys "$parent_report") <(report_keys "$candidate_report")
[[ "$(execution_runnable "$candidate_output")" \
        == "$(read_value candidate_tag_runnable)" \
    && "$(report_keys "$candidate_report" | sha256_stream)" \
        == "$(read_value universe_keys_sha256)" ]] || {
    echo 'error: candidate tag vector drifted' >&2
    exit 1
}

if ! awk -F'\t' -v OFS='\t' '
    NR == FNR {
        if (/^#/ || ($1 == "path" && $2 == "variant")) next
        key=$1 SUBSEP $2
        if (key in before) exit 2
        for (i=1; i<=10; i++) old[key, i]=$i
        if ($5 != "parse" || $6 != "SyntaxError" ||
            $7 != "unsupported-feature" || $8 != "selection" ||
            $9 != "EngineCapability" ||
            $10 != "quickjs-oxide does not declare Test262 feature support: rest-parameters") {
            exit 3
        }
        before[key]=1
        count++
        next
    }
    /^#/ || ($1 == "path" && $2 == "variant") { next }
    {
        key=$1 SUBSEP $2
        if (!(key in before) || key in after) exit 4
        for (i=1; i<=6; i++) if ($i != old[key, i]) exit 5
        if ($7 != "pass" || $8 != "parse" || $9 != "SyntaxError") exit 6
        after[key]=1
        after_count++
    }
    END {
        if (count != 192 || after_count != 192) exit 7
        for (key in before) if (!(key in after)) exit 8
    }
' "$parent_report" "$candidate_report"; then
    echo 'error: exact 192-row activation transition drifted' >&2
    exit 1
fi

transition_tmp=$(mktemp "$transition_receipt.XXXXXX")
{
    printf '# Exhaustive pinned Test262 rest-parameters global admission transition.\n'
    printf '# before_oxide_profile_sha256=%s\n' \
        "$(read_value parent_oxide_profile_sha256)"
    printf '# after_oxide_profile_sha256=%s\n' \
        "$(read_value candidate_oxide_profile_sha256)"
    printf '# manifest_sha256=%s\n' "$(read_value universe_sha256)"
    printf 'path\tvariant\tflags\tfeatures\texpected_phase\texpected_type\tbefore_outcome\tbefore_actual_phase\tbefore_actual_type\tbefore_detail\tafter_outcome\tafter_actual_phase\tafter_actual_type\tafter_detail\n'
    awk -F'\t' -v OFS='\t' '
        NR == FNR {
            if (!/^#/ && !($1 == "path" && $2 == "variant")) {
                key=$1 SUBSEP $2
                if (key in before) exit 2
                for (i=1; i<=10; i++) old[key, i]=$i
                before[key]=1
            }
            next
        }
        !/^#/ && !($1 == "path" && $2 == "variant") {
            key=$1 SUBSEP $2
            if (!(key in before) || key in after) exit 3
            for (i=1; i<=6; i++) if ($i != old[key, i]) exit 4
            print $1, $2, $3, $4, $5, $6,
                old[key, 7], old[key, 8], old[key, 9], old[key, 10],
                $7, $8, $9, $10
            after[key]=1
        }
        END { for (key in before) if (!(key in after)) exit 5 }
    ' "$parent_report" "$candidate_report"
} >"$transition_tmp"
[[ "$(awk -F'\t' '!/^#/ && !($1 == "path" && $2 == "variant") { n++ } END { print n + 0 }' "$transition_tmp")" \
        == "$(read_value transition_rows)" ]] || {
    echo 'error: transition receipt cardinality drifted' >&2
    exit 1
}

transition_sha=$(sha256_file "$transition_tmp")
transition_data_sha=$(awk '!/^#/ && !/^path\tvariant\t/' "$transition_tmp" \
    | sha256_stream)
parent_nonpass=$(report_nonpass_sha256 "$parent_report")
candidate_nonpass=$(report_nonpass_sha256 "$candidate_report")
parent_tsv=$(sha256_file "$parent_report")
parent_jsonl=$(sha256_file "$parent_json_report")
candidate_tsv=$(sha256_file "$candidate_report")
candidate_jsonl=$(sha256_file "$candidate_json_report")

if [[ "$mode" == bless-tag ]]; then
    chmod 644 "$transition_tmp"
    mv -- "$transition_tmp" "$transition_receipt"
    transition_tmp=
    update_values "$baseline" \
        "focused_baseline_sha256=$focused_sha" \
        "transition_receipt_sha256=$transition_sha" \
        "transition_data_sha256=$transition_data_sha" \
        "parent_tag_nonpass_sha256=$parent_nonpass" \
        "parent_tag_tsv_sha256=$parent_tsv" \
        "parent_tag_jsonl_sha256=$parent_jsonl" \
        "candidate_tag_nonpass_sha256=$candidate_nonpass" \
        "candidate_tag_tsv_sha256=$candidate_tsv" \
        "candidate_tag_jsonl_sha256=$candidate_jsonl"
    echo 'rest-parameters 192-row tag transition blessed'
    exit 0
fi

for entry in \
    "transition_receipt_sha256:$transition_sha" \
    "transition_data_sha256:$transition_data_sha" \
    "parent_tag_nonpass_sha256:$parent_nonpass" \
    "parent_tag_tsv_sha256:$parent_tsv" \
    "parent_tag_jsonl_sha256:$parent_jsonl" \
    "candidate_tag_nonpass_sha256:$candidate_nonpass" \
    "candidate_tag_tsv_sha256:$candidate_tsv" \
    "candidate_tag_jsonl_sha256:$candidate_jsonl"; do
    key=${entry%%:*}
    actual=${entry#*:}
    [[ "$actual" == "$(read_value "$key")" ]] || {
        printf 'error: tag receipt drifted: %s\n' "$key" >&2
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
    echo 'rest-parameters tag transition exact: 192 unsupported-feature -> pass'
    exit 0
fi

# --full always executes fresh parent and candidate receipts, then proves a
# complete key join. No projected or inherited candidate receipt is accepted.
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
candidate_full_tsv=$(sha256_file "$candidate_full_report")
candidate_full_jsonl=$(sha256_file "$candidate_full_json_report")
[[ "$(execution_runnable "$candidate_full_output")" \
        == "$(read_value candidate_full_runnable)" \
    && "$(report_outcome_count "$candidate_full_report" pass)" \
        == "$(read_value candidate_full_passes)" \
    && "$(report_outcome_count "$candidate_full_report" unsupported-feature)" \
        == "$(read_value candidate_full_unsupported_feature)" \
    && "$(unsupported_total "$(report_summary "$candidate_full_report")")" \
        == "$(read_value candidate_full_total_unsupported)" \
    && "$(report_keys "$candidate_full_report" | sha256_stream)" \
        == "$(read_value full_keys_sha256)" ]] || {
    echo 'error: candidate full vector drifted' >&2
    exit 1
}
if [[ "$full_bootstrap" == 1 && "$canonical_state" == candidate ]]; then
    [[ "$(read_value_from "$canonical_baseline" tsv_sha256)" \
            == "$candidate_full_tsv" \
        && "$(read_value_from "$canonical_baseline" jsonl_sha256)" \
            == "$candidate_full_jsonl" ]] || {
        echo 'error: bootstrap candidate does not match the canonical receipt hashes' >&2
        exit 1
    }
fi
diff -u <(report_keys "$parent_full_report") \
    <(report_keys "$candidate_full_report")

parent_full_universe=$(rows_for_paths "$universe_manifest" "$parent_full_report")
candidate_full_universe=$(rows_for_paths "$universe_manifest" "$candidate_full_report")
parent_non_universe=$(rows_without_paths "$universe_manifest" "$parent_full_report")
candidate_non_universe=$(rows_without_paths "$universe_manifest" "$candidate_full_report")
parent_full_universe_json=$(json_rows_for_paths "$universe_manifest" \
    "$parent_full_json_report")
candidate_full_universe_json=$(json_rows_for_paths "$universe_manifest" \
    "$candidate_full_json_report")
parent_non_universe_json=$(json_rows_without_paths "$universe_manifest" \
    "$parent_full_json_report")
candidate_non_universe_json=$(json_rows_without_paths "$universe_manifest" \
    "$candidate_full_json_report")
[[ "$(printf '%s\n' "$parent_full_universe" | wc -l | tr -d '[:space:]')" \
        == "$(read_value full_universe_rows)" \
    && "$(printf '%s\n' "$candidate_full_universe" | wc -l | tr -d '[:space:]')" \
        == "$(read_value full_universe_rows)" \
    && "$(printf '%s\n' "$parent_non_universe" | wc -l | tr -d '[:space:]')" \
        == "$(read_value full_non_universe_rows)" \
    && "$(printf '%s\n' "$candidate_non_universe" | wc -l | tr -d '[:space:]')" \
        == "$(read_value full_non_universe_rows)" ]] || {
    echo 'error: full universe partition drifted' >&2
    exit 1
}
diff -u <(report_rows "$parent_report") <(printf '%s\n' "$parent_full_universe")
diff -u <(report_rows "$candidate_report") \
    <(printf '%s\n' "$candidate_full_universe")
diff -u <(printf '%s\n' "$parent_non_universe") \
    <(printf '%s\n' "$candidate_non_universe")
diff -u <(json_rows_for_paths "$universe_manifest" "$parent_json_report") \
    <(printf '%s\n' "$parent_full_universe_json")
diff -u <(json_rows_for_paths "$universe_manifest" "$candidate_json_report") \
    <(printf '%s\n' "$candidate_full_universe_json")
diff -u <(printf '%s\n' "$parent_non_universe_json") \
    <(printf '%s\n' "$candidate_non_universe_json")

changed_keys_tmp=$(mktemp target/test262-rest-parameters-global-changed.XXXXXX)
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
            if (!(old[key, 7] == "unsupported-feature" && $7 == "pass")) {
                invalid_change++
            }
        } else {
            unchanged++
        }
        after[key]=1
        after_count++
    }
    END {
        if (before_count != after_count) exit 5
        for (key in before) if (!(key in after)) exit 6
        print before_count + 0, changes + 0, outcome_changes + 0,
            detail_only + 0, unchanged + 0, regressions + 0,
            invalid_change + 0
    }
' "$parent_full_report" "$candidate_full_report") || {
    echo 'error: complete full keyed join failed' >&2
    exit 1
}
read -r full_rows changed_rows outcome_changed_rows detail_only_rows \
    unchanged_rows regressions invalid_changes <<<"$join_counts"
[[ "$full_rows" == "$(read_value full_variants)" \
    && "$changed_rows" == "$(read_value full_changed_rows)" \
    && "$outcome_changed_rows" == "$(read_value full_outcome_changed_rows)" \
    && "$detail_only_rows" == "$(read_value full_detail_only_rows)" \
    && "$unchanged_rows" == "$(read_value full_unchanged_rows)" \
    && "$regressions" == "$(read_value previous_pass_regressions)" \
    && "$invalid_changes" == 0 ]] || {
    echo 'error: full keyed join counts drifted' >&2
    exit 1
}
diff -u "$derived_keys_tmp" "$changed_keys_tmp"

parent_universe_tsv_sha=$(printf '%s\n' "$parent_full_universe" | sha256_stream)
candidate_universe_tsv_sha=$(printf '%s\n' "$candidate_full_universe" | sha256_stream)
parent_universe_json_sha=$(printf '%s\n' "$parent_full_universe_json" | sha256_stream)
candidate_universe_json_sha=$(printf '%s\n' "$candidate_full_universe_json" | sha256_stream)
non_universe_tsv_sha=$(printf '%s\n' "$parent_non_universe" | sha256_stream)
non_universe_json_sha=$(printf '%s\n' "$parent_non_universe_json" | sha256_stream)

if [[ "$mode" == bless-full ]]; then
    update_values "$baseline" \
        "candidate_full_tsv_sha256=$candidate_full_tsv" \
        "candidate_full_jsonl_sha256=$candidate_full_jsonl" \
        "full_parent_universe_tsv_data_sha256=$parent_universe_tsv_sha" \
        "full_parent_universe_jsonl_data_sha256=$parent_universe_json_sha" \
        "full_candidate_universe_tsv_data_sha256=$candidate_universe_tsv_sha" \
        "full_candidate_universe_jsonl_data_sha256=$candidate_universe_json_sha" \
        "full_non_universe_tsv_data_sha256=$non_universe_tsv_sha" \
        "full_non_universe_jsonl_data_sha256=$non_universe_json_sha"
    echo 'rest-parameters strict full receipt blessed after exact 102037-row join'
    exit 0
fi

for entry in \
    "candidate_full_tsv_sha256:$candidate_full_tsv" \
    "candidate_full_jsonl_sha256:$candidate_full_jsonl" \
    "full_parent_universe_tsv_data_sha256:$parent_universe_tsv_sha" \
    "full_parent_universe_jsonl_data_sha256:$parent_universe_json_sha" \
    "full_candidate_universe_tsv_data_sha256:$candidate_universe_tsv_sha" \
    "full_candidate_universe_jsonl_data_sha256:$candidate_universe_json_sha" \
    "full_non_universe_tsv_data_sha256:$non_universe_tsv_sha" \
    "full_non_universe_jsonl_data_sha256:$non_universe_json_sha"; do
    key=${entry%%:*}
    actual=${entry#*:}
    [[ "$actual" == "$(read_value "$key")" ]] || {
        printf 'error: full receipt drifted: %s\n' "$key" >&2
        exit 1
    }
done

echo 'rest-parameters full transition exact: 192 outcome changes, 101845 byte-identical rows, zero regressions'
