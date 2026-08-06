#!/usr/bin/env bash
# Reproduce the R3dm Test262 agent Stage A global admission.

set -euo pipefail
export LC_ALL=C
export TZ=America/Los_Angeles

script_dir=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd)
root=$(CDPATH='' cd -- "$script_dir/.." && pwd)
baseline=tests/test262-agent-stage-a-global-baseline.txt
predecessor_baseline=tests/test262-shared-atomics-global-baseline.txt
scoped_baseline=tests/test262-agent-stage-a-baseline.txt
scoped_gate=scripts/test-test262-agent-stage-a.sh
canonical_baseline=tests/test262-full-baseline.txt
upstream=compat/upstream.toml
live_profile=compat/test262-oxide.conf
parent_profile=tests/test262-agent-stage-a-global-parent.conf
candidate_profile=tests/test262-agent-stage-a-global-candidate.conf
manifest=tests/test262-agent-stage-a-universe.txt
activation=tests/test262-agent-stage-a.txt
retained=tests/test262-agent-stage-a-retained.txt
parent_report=tests/test262-agent-stage-a-global-parent.tsv
candidate_report=tests/test262-agent-stage-a-global-candidate.tsv
transition=tests/test262-agent-stage-a-global-transitions.tsv
parent_replay=target/test262-agent-stage-a-global-parent-replay.tsv
candidate_replay=target/test262-agent-stage-a-global-candidate-replay.tsv
preferred_parent_full=${TEST262_AGENT_STAGE_A_PARENT_FULL:-target/test262-shared-atomics-global-candidate-full.tsv}
generated_parent_full=target/test262-agent-stage-a-global-parent-full.tsv
candidate_full=target/test262-agent-stage-a-global-candidate-full.tsv
oracle_log=target/test262-agent-stage-a-global-quickjs.log
workers=${TEST262_WORKERS:-8}
full_workers=${TEST262_FULL_WORKERS:-2}
reuse_full_reports=${TEST262_REUSE_FULL_REPORTS:-false}
runner_override=${TEST262_RUNNER:-}

baseline_lines=116
baseline_sha=d030977615eba3015fd80b0ad18a90c9cbe6c5e9cebd28c1692fcde9ad488c7c

usage() {
    printf 'usage: %s [--check|--full]\n' "${0##*/}"
    printf '  --check  authenticate profiles, focused receipts, metadata, and QuickJS\n'
    printf '  default  additionally replay the exact 59-path / 118-variant transition\n'
    printf '  --full   additionally replay and join the authenticated 102037-row reports\n'
}

mode=focused
case ${1-} in
    '') ;;
    --check) mode=check ;;
    --full) mode=full ;;
    -h|--help) usage; exit 0 ;;
    *) usage >&2; exit 2 ;;
esac
[[ $# -le 1 ]] || { usage >&2; exit 2; }
[[ "$workers" =~ ^[1-9][0-9]*$ && "$full_workers" =~ ^[1-9][0-9]*$ ]] \
    || { echo 'error: invalid Test262 worker count' >&2; exit 2; }
[[ "$reuse_full_reports" == false || "$reuse_full_reports" == true ]] \
    || { echo 'error: TEST262_REUSE_FULL_REPORTS must be true or false' >&2; exit 2; }

die() { echo "error: $*" >&2; exit 1; }
sha() {
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum "$1" | awk '{print $1}'
    else
        shasum -a 256 "$1" | awk '{print $1}'
    fi
}
lines() { wc -l <"$1" | tr -d '[:space:]'; }
value_from() {
    awk -F= -v wanted="$2" \
        '$1==wanted{sub(/^[^=]*=/,"");print;found++} END{if(found!=1)exit 1}' \
        "$1"
}
value() { value_from "$baseline" "$1"; }
predecessor_value() { value_from "$predecessor_baseline" "$1"; }
scoped_value() { value_from "$scoped_baseline" "$1"; }
canonical_value() { value_from "$canonical_baseline" "$1"; }
header() {
    awk -F= -v wanted="# $2" \
        '$1==wanted{sub(/^[^=]*=/,"");print;found++} END{if(found!=1)exit 1}' \
        "$1"
}
check_file() {
    [[ -f "$1" && "$(lines "$1")" == "$2" && "$(sha "$1")" == "$3" ]] \
        || die "authenticated input drifted: $1"
}
profile_section() {
    awk -v wanted="[$1]" '
        $0==wanted{inside=1;next} /^\[/{inside=0}
        inside&&NF&&$1!~/^#/{print}
    ' "$2"
}
report_rows() { awk -F'\t' '!/^#/&&!($1=="path"&&$2=="variant")' "$1"; }
report_keys() { report_rows "$1" | awk -F'\t' '{print $1 "\t" $2}' | sort; }
json_result_rows() { awk '/^\{"kind":"result"/' "$1"; }
report_summary() { tail -n 1 "$1" | sed 's/^# summary //'; }
computed_summary() {
    report_rows "$1" | awk -F'\t' '{print $7}' | sort | uniq -c | awk '
        {out=out (NR==1?"":" ") $2 "=" $1} END{print out}'
}
report_count() {
    report_rows "$2" | awk -F'\t' -v wanted="$1" \
        '$7==wanted{count++} END{print count+0}'
}
report_runnable() {
    report_rows "$1" | awk -F'\t' '$8!="selection"{count++} END{print count+0}'
}
rows_for_paths() {
    awk -F'\t' 'NR==FNR{if(NF&&$1!~/^#/)wanted[$1]=1;next}
        !/^#/&&!($1=="path"&&$2=="variant")&&($1 in wanted)' "$1" "$2"
}
rows_without_paths() {
    awk -F'\t' 'NR==FNR{if(NF&&$1!~/^#/)wanted[$1]=1;next}
        !/^#/&&!($1=="path"&&$2=="variant")&&!($1 in wanted)' "$1" "$2"
}
json_rows_for_paths() {
    awk 'NR==FNR{if(NF&&$1!~/^#/)wanted[$1]=1;next}
        /^\{"kind":"result"/{
            if(!match($0,/"path":"[^"]*"/))exit 2
            path=substr($0,RSTART+8,RLENGTH-9)
            if(path in wanted)print
        }' "$1" "$2"
}
json_rows_without_paths() {
    awk 'NR==FNR{if(NF&&$1!~/^#/)wanted[$1]=1;next}
        /^\{"kind":"result"/{
            if(!match($0,/"path":"[^"]*"/))exit 2
            path=substr($0,RSTART+8,RLENGTH-9)
            if(!(path in wanted))print
        }' "$1" "$2"
}
toml_test262_value() {
    awk -v wanted="$1" '
        $0=="[test262]"{inside=1;next} /^\[/{inside=0}
        inside{
            separator=index($0,"=");if(!separator)next
            key=substr($0,1,separator-1);gsub(/^[[:space:]]+|[[:space:]]+$/, "", key)
            if(key!=wanted)next
            answer=substr($0,separator+1);gsub(/^[[:space:]]+|[[:space:]]+$/, "", answer)
            if(answer~/^".*"$/)answer=substr(answer,2,length(answer)-2)
            print answer;found++
        }
        END{if(found!=1)exit 1}
    ' "$upstream"
}
full_receipt_is_blessed() {
    [[ "$(value canonical_candidate_sha256)" != PENDING_FULL \
        && "$(value candidate_full_tsv_sha256)" != PENDING_FULL \
        && "$(value candidate_full_jsonl_sha256)" != PENDING_FULL ]]
}
check_full_receipt_state() {
    local entry pending=0 frozen=0
    for entry in canonical_candidate_sha256 candidate_full_tsv_sha256 \
        candidate_full_jsonl_sha256; do
        if [[ "$(value "$entry")" == PENDING_FULL ]]; then
            pending=$((pending + 1))
        else
            frozen=$((frozen + 1))
        fi
    done
    [[ "$pending" == 0 || "$frozen" == 0 ]] \
        || die 'R3dm canonical and full receipt hashes are only partially frozen'
}

verify_report() {
    local report=$1 profile_sha=$2 label=$3 json=${1%.tsv}.jsonl
    [[ -f "$report" && -f "$json" \
        && "$(header "$report" quickjs)" == "$(value quickjs)" \
        && "$(header "$report" test262)" == "$(value test262)" \
        && "$(header "$report" test262_patch_sha256)" == "$(value test262_patch_sha256)" \
        && "$(header "$report" test262_config_sha256)" == "$(value test262_config_sha256)" \
        && "$(header "$report" test262_metadata_sha256)" == "$(value test262_metadata_sha256)" \
        && "$(header "$report" oxide_profile_sha256)" == "$profile_sha" \
        && "$(header "$report" profile)" == "$(value schema)" \
        && "$(header "$report" mode)" == "$(value mode)" \
        && "$(report_rows "$report" | lines /dev/stdin)" == "$(value manifest_variants)" \
        && "$(report_keys "$report" | sha /dev/stdin)" == "$(value manifest_keys_sha256)" \
        && "$(report_rows "$report" | sha /dev/stdin)" == "$(value "${label}_rows_sha256")" \
        && "$(json_result_rows "$json" | lines /dev/stdin)" == "$(value manifest_variants)" \
        && "$(json_result_rows "$json" | sha /dev/stdin)" == "$(value "${label}_json_rows_sha256")" \
        && "$(report_summary "$report")" == "$(value "${label}_summary")" \
        && "$(computed_summary "$report")" == "$(value "${label}_summary")" \
        && "$(sha "$report")" == "$(value "${label}_tsv_sha256")" \
        && "$(sha "$json")" == "$(value "${label}_jsonl_sha256")" ]] \
        || die "focused report drifted: $report"
}

verify_full_report() {
    local report=$1 profile_sha=$2 label=$3 json=${1%.tsv}.jsonl
    [[ -f "$report" && -f "$json" \
        && "$(header "$report" quickjs)" == "$(value quickjs)" \
        && "$(header "$report" test262)" == "$(value test262)" \
        && "$(header "$report" test262_patch_sha256)" == "$(value test262_patch_sha256)" \
        && "$(header "$report" test262_config_sha256)" == "$(value test262_config_sha256)" \
        && "$(header "$report" test262_metadata_sha256)" == "$(value test262_metadata_sha256)" \
        && "$(header "$report" oxide_profile_sha256)" == "$profile_sha" \
        && "$(header "$report" profile)" == "$(value schema)" \
        && "$(header "$report" mode)" == "$(value mode)" \
        && "$(lines "$report")" == "$(value full_tsv_lines)" \
        && "$(lines "$json")" == "$(value full_jsonl_lines)" \
        && "$(report_rows "$report" | lines /dev/stdin)" == "$(value full_variants)" \
        && "$(report_keys "$report" | sha /dev/stdin)" == "$(value full_keys_sha256)" \
        && "$(report_summary "$report")" == "$(value "${label}_summary")" \
        && "$(computed_summary "$report")" == "$(value "${label}_summary")" \
        && "$(sha "$report")" == "$(value "${label}_tsv_sha256")" \
        && "$(sha "$json")" == "$(value "${label}_jsonl_sha256")" ]] \
        || die "full report drifted: $report"
}

make_transition() {
    local before=$1 after=$2 output=$3
    {
        echo '# Exhaustive R3dm Test262 agent Stage A global admission transition.'
        echo "# parent_commit=$(value parent_commit)"
        echo "# parent_profile_sha256=$(value parent_profile_sha256)"
        echo "# candidate_profile_sha256=$(value candidate_profile_sha256)"
        echo "# manifest_sha256=$(value manifest_sha256)"
        printf 'path\tvariant\tflags\tfeatures\texpected_phase\texpected_type\tbefore_outcome\tbefore_actual_phase\tbefore_actual_type\tbefore_detail\tafter_outcome\tafter_actual_phase\tafter_actual_type\tafter_detail\n'
        awk -F'\t' 'BEGIN{OFS="\t"}
            NR==FNR{
                if(!/^#/&&!($1=="path"&&$2=="variant")){
                    key=$1 FS $2;if(key in old)exit 2;old[key]=$0
                }
                next
            }
            !/^#/&&!($1=="path"&&$2=="variant"){
                key=$1 FS $2;if(!(key in old)||key in seen)exit 3
                split(old[key],a,FS);for(i=1;i<=6;i++)if(a[i]!=$i)exit 4
                print $1,$2,$3,$4,$5,$6,a[7],a[8],a[9],a[10],$7,$8,$9,$10
                seen[key]=1
            }
            END{for(key in old)if(!(key in seen))exit 5}
        ' "$before" "$after"
    } >"$output"
}

transition_counts() {
    awk -F'\t' '!/^#/&&!($1=="path"&&$2=="variant"){
        different=0;for(i=7;i<=10;i++)if($i!=$(i+4))different=1
        if($7!="pass"&&$11=="pass")gain++
        if($7=="pass"&&$11!="pass")regress++
        if(different){changed++;if($7!=$11)outcome++;else detail++}else unchanged++
    } END{printf "changed=%d outcome=%d detail=%d unchanged=%d gains=%d regressions=%d",changed,outcome,detail,unchanged,gain,regress}' "$1"
}

check_profiles() {
    check_file "$parent_profile" "$(value parent_profile_lines)" \
        "$(value parent_profile_sha256)"
    check_file "$candidate_profile" "$(value candidate_profile_lines)" \
        "$(value candidate_profile_sha256)"
    check_file "$live_profile" "$(value candidate_profile_lines)" \
        "$(value candidate_profile_sha256)"
    cmp -s "$candidate_profile" "$live_profile" \
        || die 'live profile is not byte-identical to the R3dm candidate'

    local section
    for section in features audited-negative-tests execution host-agent-tests; do
        profile_section "$section" "$parent_profile" >"$tmp/parent.$section"
        profile_section "$section" "$candidate_profile" >"$tmp/candidate.$section"
    done
    [[ "$(lines "$tmp/parent.features")" == "$(value parent_features)" \
        && "$(sha "$tmp/parent.features")" == "$(value parent_features_sha256)" \
        && "$(lines "$tmp/candidate.features")" == "$(value candidate_features)" \
        && "$(sha "$tmp/candidate.features")" == "$(value candidate_features_sha256)" \
        && "$(lines "$tmp/parent.audited-negative-tests")" == "$(value audited_negative_tests)" \
        && "$(sha "$tmp/parent.audited-negative-tests")" == "$(value audited_negative_tests_sha256)" \
        && "$(lines "$tmp/parent.execution")" == "$(value execution_entries)" \
        && "$(sha "$tmp/parent.execution")" == "$(value execution_sha256)" \
        && ! -s "$tmp/parent.host-agent-tests" \
        && "$(lines "$tmp/candidate.host-agent-tests")" == \
            "$(value candidate_agent_allowlist_paths)" \
        && "$(sha "$tmp/candidate.host-agent-tests")" == \
            "$(value candidate_agent_allowlist_sha256)" ]] \
        || die 'R3dm profile inventory drifted'
    diff -u "$tmp/parent.features" "$tmp/candidate.features"
    diff -u "$tmp/parent.audited-negative-tests" "$tmp/candidate.audited-negative-tests"
    diff -u "$tmp/parent.execution" "$tmp/candidate.execution"
    diff -u "$activation" "$tmp/candidate.host-agent-tests"
}

check_manifest_and_metadata() {
    check_file "$manifest" "$(value manifest_paths)" "$(value manifest_sha256)"
    check_file "$activation" "$(value activation_paths)" \
        "$(value activation_sha256)"
    check_file "$retained" "$(value retained_paths)" "$(value retained_sha256)"
    check_file tests/test262-atomics-universe.tsv "$(value atomics_ledger_lines)" \
        "$(value atomics_ledger_sha256)"

    local file path source
    for file in "$manifest" "$activation" "$retained"; do
        sort -c "$file" || die "R3dm manifest is not bytewise sorted: $file"
        [[ -z "$(uniq -d "$file")" ]] \
            || die "R3dm manifest contains duplicates: $file"
    done
    awk -F'\t' '$2=="shared-agent"{print $1}' \
        tests/test262-atomics-universe.tsv >"$tmp/reconstructed-universe.txt"
    diff -u "$manifest" "$tmp/reconstructed-universe.txt"
    sort -u "$activation" "$retained" >"$tmp/partition.txt"
    diff -u "$manifest" "$tmp/partition.txt"
    [[ -z "$(comm -12 "$activation" "$retained")" \
        && "$(printf 'activation=%s retained=%s' "$(lines "$activation")" \
            "$(lines "$retained")")" == "$(value manifest_partition)" ]] \
        || die 'R3dm admission partition drifted'

    "$runner" --suite "$suite" --validate-metadata "$tmp/metadata.bin" >/dev/null
    [[ "$(lines "$tmp/metadata.bin")" == "$(value test262_metadata_records)" \
        && "$(sha "$tmp/metadata.bin")" == "$(value test262_metadata_sha256)" ]] \
        || die 'pinned Test262 metadata drifted'
    path=$(head -n 1 "$activation")
    source=$suite/$path
    check_file "$source" 66 "$(value activation_source_sha256)"
    grep -Fq '$262.agent.start' "$source" \
        && grep -Fq '$262.agent.report' "$source" \
        && grep -Fq '$262.agent.getReport' "$source" \
        && grep -Fq '$262.agent.leaving' "$source" \
        || die 'R3dm activation source shape drifted'
    ! grep -Eq '\$262\.agent\.(broadcast|receiveBroadcast)|Atomics\.waitAsync' "$source" \
        || die 'R3dm activation leaked later agent semantics'
}

check_receipts() {
    check_file "$parent_report" "$(value parent_report_lines)" \
        "$(value parent_tsv_sha256)"
    check_file "${parent_report%.tsv}.jsonl" "$(value parent_jsonl_lines)" \
        "$(value parent_jsonl_sha256)"
    check_file "$candidate_report" "$(value candidate_report_lines)" \
        "$(value candidate_tsv_sha256)"
    check_file "${candidate_report%.tsv}.jsonl" "$(value candidate_jsonl_lines)" \
        "$(value candidate_jsonl_sha256)"
    verify_report "$parent_report" "$(value parent_profile_sha256)" parent
    verify_report "$candidate_report" "$(value candidate_profile_sha256)" candidate

    [[ "$(report_runnable "$parent_report")" == "$(value parent_runnable)" \
        && "$(report_count pass "$parent_report")" == "$(value parent_passes)" \
        && "$(report_count unsupported-host-agent "$parent_report")" == \
            "$(value parent_unsupported_host_agent)" \
        && "$(report_runnable "$candidate_report")" == "$(value candidate_runnable)" \
        && "$(report_count pass "$candidate_report")" == "$(value candidate_passes)" \
        && "$(report_count unsupported-host-agent "$candidate_report")" == \
            "$(value candidate_unsupported_host_agent)" ]] \
        || die 'R3dm focused outcome counts drifted'

    rows_for_paths "$activation" "$parent_report" >"$tmp/parent.activation"
    rows_for_paths "$activation" "$candidate_report" >"$tmp/candidate.activation"
    rows_for_paths "$retained" "$parent_report" >"$tmp/parent.retained"
    rows_for_paths "$retained" "$candidate_report" >"$tmp/candidate.retained"
    [[ "$(lines "$tmp/parent.activation")" == "$(value activation_variants)" \
        && "$(lines "$tmp/candidate.activation")" == "$(value activation_variants)" \
        && "$(lines "$tmp/parent.retained")" == "$(value retained_variants)" \
        && "$(lines "$tmp/candidate.retained")" == "$(value retained_variants)" ]] \
        || die 'R3dm focused report partition drifted'

    awk -F'\t' '{
        if($2!~/^(sloppy|strict)$/||$7!="unsupported-host-agent"||
           $8!="selection"||$9!="HostCapability"||
           $10!="missing execution capabilities: agent")exit 2
    }' "$tmp/parent.activation" \
        || die 'R3dm parent activation frontier drifted'
    awk -F'\t' '{if($7!="pass"||$8!="normal"||$9!=""||$10!="")exit 2}' \
        "$tmp/candidate.activation" \
        || die 'R3dm candidate activation outcomes drifted'
    awk -F'\t' '{
        if($7!="unsupported-host-agent"||$8!="selection"||
           $9!="HostCapability"||$10!="missing execution capabilities: agent")exit 2
    }' "$tmp/parent.retained" "$tmp/candidate.retained" \
        || die 'R3dm retained agent diagnostics drifted'

    check_file "$transition" "$(value transition_lines)" "$(value transition_sha256)"
    [[ "$(header "$transition" parent_commit)" == "$(value parent_commit)" \
        && "$(header "$transition" parent_profile_sha256)" == "$(value parent_profile_sha256)" \
        && "$(header "$transition" candidate_profile_sha256)" == "$(value candidate_profile_sha256)" \
        && "$(header "$transition" manifest_sha256)" == "$(value manifest_sha256)" \
        && "$(report_rows "$transition" | sha /dev/stdin)" == "$(value transition_data_sha256)" \
        && "$(transition_counts "$transition")" == \
            "changed=$(value transition_changed) outcome=$(value transition_outcome_changed) detail=$(value transition_detail_only) unchanged=$(value transition_unchanged) gains=$(value transition_pass_gains) regressions=$(value transition_pass_regressions)" ]] \
        || die 'R3dm focused transition drifted'
}

check_history_and_upstream() {
    check_file "$predecessor_baseline" "$(value predecessor_baseline_lines)" \
        "$(value predecessor_baseline_sha256)"
    check_file "$scoped_baseline" "$(value scoped_baseline_lines)" \
        "$(value scoped_baseline_sha256)"
    [[ "$(predecessor_value candidate_profile_sha256)" == "$(value parent_profile_sha256)" \
        && "$(predecessor_value candidate_features)" == "$(value parent_features)" \
        && "$(predecessor_value candidate_features_sha256)" == "$(value parent_features_sha256)" \
        && "$(predecessor_value full_variants)" == "$(value full_variants)" \
        && "$(predecessor_value full_keys_sha256)" == "$(value full_keys_sha256)" \
        && "$(predecessor_value candidate_full_runnable)" == "$(value parent_full_runnable)" \
        && "$(predecessor_value candidate_full_passes)" == "$(value parent_full_passes)" \
        && "$(predecessor_value candidate_full_unsupported_feature)" == \
            "$(value parent_full_unsupported_feature)" \
        && "$(predecessor_value candidate_full_tsv_sha256)" == \
            "$(value parent_full_tsv_sha256)" \
        && "$(predecessor_value candidate_full_jsonl_sha256)" == \
            "$(value parent_full_jsonl_sha256)" \
        && "$(predecessor_value candidate_full_summary)" == \
            "$(value parent_full_summary)" \
        && "$(scoped_value universe_paths)" == "$(value manifest_paths)" \
        && "$(scoped_value universe_variants)" == "$(value manifest_variants)" \
        && "$(scoped_value universe_sha256)" == "$(value manifest_sha256)" \
        && "$(scoped_value activation_sha256)" == "$(value activation_sha256)" \
        && "$(scoped_value retained_sha256)" == "$(value retained_sha256)" \
        && "$(scoped_value candidate_passes)" == 2 \
        && "$(scoped_value candidate_retained_unsupported)" == 116 \
        && -x "$scoped_gate" \
        && "$(toml_test262_value repository)" == https://github.com/tc39/test262.git \
        && "$(toml_test262_value commit)" == "$(value test262)" \
        && "$(toml_test262_value patch_sha256)" == "$(value test262_patch_sha256)" \
        && "$(toml_test262_value config_sha256)" == "$(value test262_config_sha256)" \
        && "$(toml_test262_value test_count)" == "$(value test262_metadata_records)" \
        && "$(toml_test262_value metadata_records_sha256)" == "$(value test262_metadata_sha256)" \
        && "$(toml_test262_value oxide_profile)" == "$live_profile" \
        && "$(toml_test262_value oxide_profile_sha256)" == "$(value candidate_profile_sha256)" ]] \
        || die 'R3dm predecessor, scoped, or upstream bridge drifted'

    if full_receipt_is_blessed; then
        check_file "$canonical_baseline" "$(value canonical_baseline_lines)" \
            "$(value canonical_candidate_sha256)"
        [[ "$(canonical_value schema)" == "$(value schema)" \
            && "$(canonical_value timeout_ms)" == "$(value timeout_ms)" \
            && "$(canonical_value variants)" == "$(value full_variants)" \
            && "$(canonical_value runnable)" == "$(value candidate_full_runnable)" \
            && "$(canonical_value passes)" == "$(value candidate_full_passes)" \
            && "$(canonical_value tsv_sha256)" == "$(value candidate_full_tsv_sha256)" \
            && "$(canonical_value jsonl_sha256)" == "$(value candidate_full_jsonl_sha256)" \
            && "$(canonical_value summary)" == "$(value candidate_full_summary)" ]] \
            || die 'R3dm candidate is not the canonical full vector'
    else
        check_file "$canonical_baseline" "$(value canonical_baseline_lines)" \
            "$(value canonical_parent_sha256)"
        [[ "$(canonical_value runnable)" == "$(value parent_full_runnable)" \
            && "$(canonical_value passes)" == "$(value parent_full_passes)" \
            && "$(canonical_value tsv_sha256)" == "$(value parent_full_tsv_sha256)" \
            && "$(canonical_value jsonl_sha256)" == "$(value parent_full_jsonl_sha256)" \
            && "$(canonical_value summary)" == "$(value parent_full_summary)" ]] \
            || die 'unblessed R3dm gate is not based on the R3dl canonical vector'
    fi
}

verify_fail_closed_selection() {
    local profile label expected rejected
    for label in parent candidate; do
        if [[ "$label" == parent ]]; then
            profile=$parent_profile
            expected=$parent_report
        else
            profile=$candidate_profile
            expected=$candidate_report
        fi
        "$runner" --suite "$suite" --config "$source_dir/test262.conf" \
            --oxide-profile "$root/$profile" --manifest "$root/$manifest" \
            --report "$tmp/$label-positive.tsv" --mode both \
            --timeout-ms "$(value timeout_ms)" --workers 1 --allow-failures \
            >/dev/null
        cmp -s "$expected" "$tmp/$label-positive.tsv" \
            && cmp -s "${expected%.tsv}.jsonl" "$tmp/$label-positive.jsonl" \
            || die "R3dm runner failed the exact $label profile handshake"

        for rejected in "$activation" "$retained" Cargo.toml; do
            if "$runner" --suite "$suite" --config "$source_dir/test262.conf" \
                --oxide-profile "$root/$profile" --manifest "$root/$rejected" \
                --report "$tmp/rejected.tsv" --mode both \
                --timeout-ms "$(value timeout_ms)" --workers 1 --allow-failures \
                >/dev/null 2>&1; then
                die "R3dm $label profile accepted a non-R3dm manifest: $rejected"
            fi
        done
        if "$runner" --suite "$suite" --config "$source_dir/test262.conf" \
            --oxide-profile "$root/$profile" \
            --test test/built-ins/Atomics/wait/good-views.js \
            --report "$tmp/rejected.tsv" --mode both \
            --timeout-ms "$(value timeout_ms)" --workers 1 --allow-failures \
            >/dev/null 2>&1; then
            die "R3dm $label profile accepted --test"
        fi
    done
}

verify_quickjs() {
    [[ -x "$source_dir/run-test262" ]] \
        || "${MAKE:-make}" -C "$source_dir" run-test262 >&2
    if ! (cd -- "$source_dir" && \
        ./run-test262 -m -c test262.conf -a -T "$workers" \
            -f test262/test/built-ins/Atomics/wait/good-views.js) \
        >"$root/$oracle_log" 2>&1; then
        tail -n 100 "$oracle_log" >&2
        die 'pinned QuickJS could not execute the R3dm activation'
    fi
    if grep -Eq '(^|[[:space:]])FAILED($|[[:space:]])|SKIPPED FEATURE' "$oracle_log" \
        || ! grep -Fq "Average memory statistics for $(value quickjs_variants) tests:" \
            "$oracle_log"; then
        tail -n 100 "$oracle_log" >&2
        die 'pinned QuickJS no longer passes both R3dm activation variants'
    fi
}

run_focused_report() {
    local profile=$1 output=$2
    "$runner" --suite "$suite" --config "$source_dir/test262.conf" \
        --oxide-profile "$root/$profile" --manifest "$root/$manifest" \
        --report "$root/$output" --mode both --timeout-ms "$(value timeout_ms)" \
        --workers "$workers" --allow-failures >/dev/null
}

replay_focused() {
    run_focused_report "$parent_profile" "$parent_replay"
    run_focused_report "$candidate_profile" "$candidate_replay"
    cmp -s "$parent_report" "$parent_replay" \
        && cmp -s "${parent_report%.tsv}.jsonl" "${parent_replay%.tsv}.jsonl" \
        || die 'R3dm parent focused replay drifted'
    cmp -s "$candidate_report" "$candidate_replay" \
        && cmp -s "${candidate_report%.tsv}.jsonl" "${candidate_replay%.tsv}.jsonl" \
        || die 'R3dm candidate focused replay drifted'
    make_transition "$parent_replay" "$candidate_replay" "$tmp/replayed-transition.tsv"
    diff -u "$transition" "$tmp/replayed-transition.tsv"
}

run_full_report() {
    local profile=$1 output=$2
    case $output in /*) ;; *) output=$root/$output ;; esac
    "$runner" --suite "$suite" --config "$source_dir/test262.conf" \
        --oxide-profile "$root/$profile" --all --report "$output" \
        --mode both --timeout-ms "$(value timeout_ms)" --workers "$full_workers" \
        --allow-failures >/dev/null
}

verify_full_join() {
    local counts expected parent_json=${parent_full%.tsv}.jsonl
    local candidate_json=${candidate_full%.tsv}.jsonl
    rows_for_paths "$activation" "$parent_full" >"$tmp/parent.scope"
    rows_for_paths "$activation" "$candidate_full" >"$tmp/candidate.scope"
    rows_without_paths "$activation" "$parent_full" >"$tmp/parent.outside"
    rows_without_paths "$activation" "$candidate_full" >"$tmp/candidate.outside"
    json_rows_for_paths "$activation" "$parent_json" >"$tmp/parent.scope.json"
    json_rows_for_paths "$activation" "$candidate_json" >"$tmp/candidate.scope.json"
    json_rows_without_paths "$activation" "$parent_json" >"$tmp/parent.outside.json"
    json_rows_without_paths "$activation" "$candidate_json" >"$tmp/candidate.outside.json"
    rows_for_paths "$activation" "$parent_report" >"$tmp/focused.parent"
    rows_for_paths "$activation" "$candidate_report" >"$tmp/focused.candidate"
    json_rows_for_paths "$activation" "${parent_report%.tsv}.jsonl" \
        >"$tmp/focused.parent.json"
    json_rows_for_paths "$activation" "${candidate_report%.tsv}.jsonl" \
        >"$tmp/focused.candidate.json"
    [[ "$(lines "$tmp/parent.scope")" == "$(value full_scope_rows)" \
        && "$(lines "$tmp/candidate.scope")" == "$(value full_scope_rows)" \
        && "$(lines "$tmp/parent.outside")" == "$(value full_outside_rows)" \
        && "$(lines "$tmp/candidate.outside")" == "$(value full_outside_rows)" ]] \
        || die 'R3dm full partition row counts drifted'
    diff -u "$tmp/focused.parent" "$tmp/parent.scope"
    diff -u "$tmp/focused.candidate" "$tmp/candidate.scope"
    diff -u "$tmp/focused.parent.json" "$tmp/parent.scope.json"
    diff -u "$tmp/focused.candidate.json" "$tmp/candidate.scope.json"
    diff -u "$tmp/parent.outside" "$tmp/candidate.outside"
    diff -u "$tmp/parent.outside.json" "$tmp/candidate.outside.json"

    counts=$(awk -F'\t' -v parent="$parent_full" '
        FILENAME==parent{
            if(!/^#/&&!($1=="path"&&$2=="variant")){
                key=$1 FS $2;if(key in old)exit 2;old[key]=$0;before++
            }
            next
        }
        !/^#/&&!($1=="path"&&$2=="variant"){
            key=$1 FS $2;if(!(key in old)||key in seen)exit 3
            split(old[key],a,FS);for(i=1;i<=6;i++)if(a[i]!=$i)exit 4
            if(a[7]=="pass"&&$7!="pass")regress++
            if(old[key]!=$0){changed++;if(a[7]!=$7)outcome++;else detail++}
            seen[key]=1
        }
        END{for(key in old)if(!(key in seen))exit 5
            printf "changed=%d outcome=%d detail=%d unchanged=%d regressions=%d",changed,outcome,detail,before-changed,regress}
    ' "$parent_full" "$candidate_full") || die 'R3dm exact full join failed'
    expected="changed=$(value full_changed) outcome=$(value full_outcome_changed) detail=$(value full_detail_only) unchanged=$(value full_unchanged) regressions=$(value full_pass_regressions)"
    [[ "$counts" == "$expected" ]] || die "R3dm full transition drifted: $counts"
}

replay_full() {
    full_receipt_is_blessed \
        || die 'R3dm full hashes are explicitly PENDING_FULL; run and bless the full receipts first'
    parent_full=$preferred_parent_full
    if [[ ! -f "$parent_full" || ! -f "${parent_full%.tsv}.jsonl" \
        || "$(sha "$parent_full")" != "$(value parent_full_tsv_sha256)" \
        || "$(sha "${parent_full%.tsv}.jsonl")" != "$(value parent_full_jsonl_sha256)" ]]; then
        parent_full=$generated_parent_full
        if [[ "$reuse_full_reports" == false \
            || ! -f "$parent_full" || ! -f "${parent_full%.tsv}.jsonl" \
            || "$(sha "$parent_full")" != "$(value parent_full_tsv_sha256)" \
            || "$(sha "${parent_full%.tsv}.jsonl")" != "$(value parent_full_jsonl_sha256)" ]]; then
            run_full_report "$parent_profile" "$parent_full"
        fi
    fi
    if [[ "$reuse_full_reports" == false \
        || ! -f "$candidate_full" || ! -f "${candidate_full%.tsv}.jsonl" \
        || "$(sha "$candidate_full")" != "$(value candidate_full_tsv_sha256)" \
        || "$(sha "${candidate_full%.tsv}.jsonl")" != "$(value candidate_full_jsonl_sha256)" ]]; then
        run_full_report "$candidate_profile" "$candidate_full"
    fi
    verify_full_report "$parent_full" "$(value parent_profile_sha256)" parent_full
    verify_full_report "$candidate_full" "$(value candidate_profile_sha256)" candidate_full
    [[ "$(report_runnable "$parent_full")" == "$(value parent_full_runnable)" \
        && "$(report_count pass "$parent_full")" == "$(value parent_full_passes)" \
        && "$(report_count unsupported-feature "$parent_full")" == "$(value parent_full_unsupported_feature)" \
        && "$(report_runnable "$candidate_full")" == "$(value candidate_full_runnable)" \
        && "$(report_count pass "$candidate_full")" == "$(value candidate_full_passes)" \
        && "$(report_count unsupported-feature "$candidate_full")" == "$(value candidate_full_unsupported_feature)" \
        && "$(report_count unsupported-host-agent "$parent_full")" == \
            "$(value parent_full_unsupported_host_agent)" \
        && "$(report_count unsupported-host-agent "$candidate_full")" == \
            "$(value candidate_full_unsupported_host_agent)" ]] \
        || die 'R3dm full outcome counts drifted'
    verify_full_join
}

cd -- "$root"
tmp=$(mktemp -d "${TMPDIR:-/tmp}/quickjs-oxide-r3dm-global.XXXXXX")
trap 'rm -rf "$tmp"' EXIT HUP INT TERM
suite=$("$script_dir/prepare-test262.sh")
source_dir=$(dirname -- "$suite")

if [[ -n "$runner_override" ]]; then
    runner=$runner_override
else
    cargo build --quiet --locked --release --bin run-test262
    target_dir=${CARGO_TARGET_DIR:-target}
    case $target_dir in
        /*) ;;
        *) target_dir=$root/$target_dir ;;
    esac
    runner=$target_dir/release/run-test262
fi
[[ -x "$runner" ]] || die "Test262 runner is not executable: $runner"

check_file "$baseline" "$baseline_lines" "$baseline_sha"
check_full_receipt_state
check_profiles
check_manifest_and_metadata
check_receipts
check_history_and_upstream
verify_fail_closed_selection
verify_quickjs
make_transition "$parent_report" "$candidate_report" "$tmp/transition.tsv"
diff -u "$transition" "$tmp/transition.tsv"

case $mode in
    check) ;;
    focused) replay_focused ;;
    full) replay_focused; replay_full ;;
esac

if [[ "$mode" == full ]]; then
    printf 'R3dm agent Stage A full: 2 new passes, 116 retained agent diagnostics, zero regressions\n'
else
    printf 'R3dm agent Stage A focused: 2/2 activated pass, 116 retained agent diagnostics\n'
fi
