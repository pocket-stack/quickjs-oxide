#!/usr/bin/env bash
# Reproduce the R3dl global SharedArrayBuffer and Atomics admission.

set -euo pipefail
export LC_ALL=C
export TZ=America/Los_Angeles

script_dir=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd)
root=$(CDPATH='' cd -- "$script_dir/.." && pwd)
baseline=tests/test262-shared-atomics-global-baseline.txt
predecessor_baseline=tests/test262-error-regexp-typedarray-global-baseline.txt
successor_baseline=tests/test262-agent-stage-a-global-baseline.txt
successor_gate=scripts/test-test262-agent-stage-a-global.sh
upstream=compat/upstream.toml
live_profile=compat/test262-oxide.conf
parent_profile=tests/test262-shared-atomics-global-parent.conf
candidate_profile=tests/test262-shared-atomics-global-candidate.conf
added_features=tests/test262-shared-atomics-global-added-features.txt
manifest=tests/test262-shared-atomics-global.txt
activation=tests/test262-shared-atomics-global-activation.txt
already_admitted=tests/test262-atomics-pause-global.txt
cross_realm_retained=tests/test262-shared-atomics-global-cross-realm-retained.txt
sab_universe=tests/test262-shared-array-buffer-universe.tsv
atomics_universe=tests/test262-atomics-universe.tsv
parent_report=tests/test262-shared-atomics-global-parent.tsv
candidate_report=tests/test262-shared-atomics-global-candidate.tsv
transition=tests/test262-shared-atomics-global-transitions.tsv
parent_replay=target/test262-shared-atomics-global-parent-replay.tsv
candidate_replay=target/test262-shared-atomics-global-candidate-replay.tsv
parent_full=target/test262-full.tsv
candidate_full=target/test262-shared-atomics-global-candidate-full.tsv
oracle_log=target/test262-shared-atomics-global-quickjs.log
workers=${TEST262_WORKERS:-8}
full_workers=${TEST262_FULL_WORKERS:-2}
reuse_full_reports=${TEST262_REUSE_FULL_REPORTS:-false}
runner_override=${TEST262_RUNNER:-}

baseline_lines=131
baseline_sha=537c03e9f3f0eeb8f43249af5e1f2774bd0f0fc11d446ac3d60319b49510f6ab
successor_lines=116
successor_sha=d030977615eba3015fd80b0ad18a90c9cbe6c5e9cebd28c1692fcde9ad488c7c

usage() {
    printf 'usage: %s [--check|--full]\n' "${0##*/}"
    printf '  --check  authenticate profiles, focused receipts, metadata, and QuickJS\n'
    printf '  default  additionally replay the exact 445-path / 886-variant transition\n'
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
successor_value() { value_from "$successor_baseline" "$1"; }
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
    [[ "$(value candidate_full_tsv_sha256)" != PENDING_FULL \
        && "$(value candidate_full_jsonl_sha256)" != PENDING_FULL ]]
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
        echo '# Exhaustive R3dl SharedArrayBuffer and Atomics global admission transition.'
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
        || die 'live profile is not byte-identical to the R3dl candidate'

    local section
    for section in features audited-negative-tests execution; do
        profile_section "$section" "$parent_profile" >"$tmp/parent.$section"
        profile_section "$section" "$candidate_profile" >"$tmp/candidate.$section"
    done
    comm -13 "$tmp/parent.features" "$tmp/candidate.features" >"$tmp/added.features"
    [[ "$(lines "$tmp/parent.features")" == "$(value parent_features)" \
        && "$(sha "$tmp/parent.features")" == "$(value parent_features_sha256)" \
        && "$(lines "$tmp/candidate.features")" == "$(value candidate_features)" \
        && "$(sha "$tmp/candidate.features")" == "$(value candidate_features_sha256)" \
        && "$(lines "$tmp/parent.audited-negative-tests")" == "$(value audited_negative_tests)" \
        && "$(sha "$tmp/parent.audited-negative-tests")" == "$(value audited_negative_tests_sha256)" \
        && "$(lines "$tmp/parent.execution")" == "$(value execution_entries)" \
        && "$(sha "$tmp/parent.execution")" == "$(value execution_sha256)" ]] \
        || die 'R3dl profile inventory drifted'
    diff -u "$added_features" "$tmp/added.features"
    [[ -z "$(comm -23 "$tmp/parent.features" "$tmp/candidate.features")" ]] \
        || die 'R3dl candidate removed a parent feature'
    diff -u "$tmp/parent.audited-negative-tests" "$tmp/candidate.audited-negative-tests"
    diff -u "$tmp/parent.execution" "$tmp/candidate.execution"
}

check_manifest_and_metadata() {
    check_file "$added_features" "$(value added_features_count)" \
        "$(value added_features_sha256)"
    check_file "$manifest" "$(value manifest_paths)" "$(value manifest_sha256)"
    check_file "$activation" "$(value activation_paths)" \
        "$(value activation_sha256)"
    check_file "$already_admitted" "$(value already_admitted_paths)" \
        "$(value already_admitted_sha256)"
    check_file "$cross_realm_retained" "$(value cross_realm_retained_paths)" \
        "$(value cross_realm_retained_sha256)"
    check_file tests/test262-atomics-non-shared-core.txt \
        "$(value atomics_non_shared_core_paths)" \
        "$(value atomics_non_shared_core_sha256)"
    check_file tests/test262-shared-array-buffer-core.txt \
        "$(value shared_array_buffer_core_paths)" \
        "$(value shared_array_buffer_core_sha256)"
    check_file tests/test262-shared-atomics-nonblocking.txt \
        "$(value shared_atomics_nonblocking_paths)" \
        "$(value shared_atomics_nonblocking_sha256)"
    check_file tests/test262-atomics-wait-nonagent-bounded.txt \
        "$(value atomics_wait_nonagent_bounded_paths)" \
        "$(value atomics_wait_nonagent_bounded_sha256)"
    check_file "$sab_universe" "$(value shared_array_buffer_universe_lines)" \
        "$(value shared_array_buffer_universe_sha256)"
    check_file "$atomics_universe" "$(value atomics_universe_lines)" \
        "$(value atomics_universe_sha256)"

    local file path source staging_cross=test/staging/sm/Atomics/cross-compartment.js
    for file in "$manifest" "$activation" "$already_admitted" \
        "$cross_realm_retained"; do
        sort -c "$file" || die "R3dl manifest is not bytewise sorted: $file"
        [[ -z "$(uniq -d "$file")" ]] \
            || die "R3dl manifest contains duplicates: $file"
    done

    sort -u tests/test262-atomics-non-shared-core.txt \
        tests/test262-shared-array-buffer-core.txt \
        tests/test262-shared-atomics-nonblocking.txt \
        tests/test262-atomics-wait-nonagent-bounded.txt \
        >"$tmp/authenticated-union.txt"
    [[ "$(lines "$tmp/authenticated-union.txt")" == \
        "$(value authenticated_union_paths)" ]] \
        || die 'R3dl authenticated milestone union drifted'
    { cat "$tmp/authenticated-union.txt"; echo "$staging_cross"; } \
        | sort -u >"$tmp/reconstructed-universe.txt"
    [[ "$(( $(lines "$tmp/reconstructed-universe.txt") - \
        $(lines "$tmp/authenticated-union.txt") ))" == \
        "$(value staging_spillover_paths)" ]] \
        || die 'R3dl staging spillover cardinality drifted'
    diff -u "$manifest" "$tmp/reconstructed-universe.txt"

    sort -u "$activation" "$already_admitted" "$cross_realm_retained" \
        >"$tmp/partition.txt"
    diff -u "$manifest" "$tmp/partition.txt"
    [[ -z "$(comm -12 "$activation" "$already_admitted")" \
        && -z "$(comm -12 "$activation" "$cross_realm_retained")" \
        && -z "$(comm -12 "$already_admitted" "$cross_realm_retained")" \
        && "$(printf 'activation=%s already-admitted=%s cross-realm-retained=%s' \
            "$(lines "$activation")" "$(lines "$already_admitted")" \
            "$(lines "$cross_realm_retained")")" == "$(value manifest_partition)" ]] \
        || die 'R3dl admission partition drifted'
    grep -Fxq "$staging_cross" "$activation" \
        || die 'R3dl staging Atomics spillover left the activation partition'
    [[ -z "$(comm -12 "$tmp/authenticated-union.txt" \
        <(printf '%s\n' "$staging_cross"))" ]] \
        || die 'R3dl staging spillover became part of an earlier milestone manifest'

    "$runner" --suite "$suite" --validate-metadata "$tmp/metadata.bin" >/dev/null
    [[ "$(lines "$tmp/metadata.bin")" == "$(value test262_metadata_records)" \
        && "$(sha "$tmp/metadata.bin")" == "$(value test262_metadata_sha256)" ]] \
        || die 'pinned Test262 metadata drifted'
    tr '\0' '\t' <"$tmp/metadata.bin" >"$tmp/metadata.tsv"
    awk -F'\t' 'NR==FNR{wanted[$1]=1;next} $1 in wanted{print $1}' \
        "$tmp/authenticated-union.txt" "$tmp/metadata.tsv" | sort -u \
        >"$tmp/metadata-union.txt"
    diff -u "$tmp/authenticated-union.txt" "$tmp/metadata-union.txt"
    awk -F'\t' '
        function has(list,value){return index("," list ",","," value ",")!=0}
        NR==FNR{wanted[$1]=1;next}
        $1 in wanted{
            if(!has($4,"SharedArrayBuffer")||!has($4,"cross-realm"))exit 2
            print $1
        }
    ' "$cross_realm_retained" "$tmp/metadata.tsv" | sort -u \
        >"$tmp/cross-realm-metadata.txt" \
        || die 'R3dl cross-realm retained metadata drifted'
    diff -u "$cross_realm_retained" "$tmp/cross-realm-metadata.txt"

    check_file "$suite/$staging_cross" 114 \
        "$(value staging_cross_compartment_sha256)"
    check_file "$suite/harness/testAtomics.js" 123 \
        "$(value test_atomics_harness_sha256)"
    check_file "$suite/harness/atomicsHelper.js" 328 \
        "$(value atomics_helper_harness_sha256)"
    grep -Fq '$262.createRealm' "$suite/$staging_cross" \
        && grep -Fq 'SharedArrayBuffer' "$suite/$staging_cross" \
        && grep -Fq 'Atomics' "$suite/$staging_cross" \
        || die 'R3dl staging Atomics spillover source shape drifted'

    while IFS= read -r path; do
        source=$suite/$path
        ! grep -Eq 'Atomics[[:space:]]*\.[[:space:]]*waitAsync|\$262[[:space:]]*\.[[:space:]]*agent' \
            "$source" || die "R3dl universe leaked agent or waitAsync source: $path"
    done <"$manifest"
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
        && "$(report_count unsupported-feature "$parent_report")" == "$(value parent_unsupported_feature)" \
        && "$(report_runnable "$candidate_report")" == "$(value candidate_runnable)" \
        && "$(report_count pass "$candidate_report")" == "$(value candidate_passes)" \
        && "$(report_count unsupported-feature "$candidate_report")" == "$(value candidate_unsupported_feature)" ]] \
        || die 'R3dl focused outcome counts drifted'

    rows_for_paths "$activation" "$parent_report" >"$tmp/parent.activation"
    rows_for_paths "$activation" "$candidate_report" >"$tmp/candidate.activation"
    rows_for_paths "$already_admitted" "$parent_report" >"$tmp/parent.prepass"
    rows_for_paths "$already_admitted" "$candidate_report" >"$tmp/candidate.prepass"
    rows_for_paths "$cross_realm_retained" "$parent_report" >"$tmp/parent.cross-realm"
    rows_for_paths "$cross_realm_retained" "$candidate_report" >"$tmp/candidate.cross-realm"
    [[ "$(lines "$tmp/parent.activation")" == "$(value activation_variants)" \
        && "$(lines "$tmp/candidate.activation")" == "$(value activation_variants)" \
        && "$(lines "$tmp/parent.prepass")" == "$(value already_admitted_variants)" \
        && "$(lines "$tmp/candidate.prepass")" == "$(value already_admitted_variants)" \
        && "$(lines "$tmp/parent.cross-realm")" == "$(value cross_realm_retained_variants)" \
        && "$(lines "$tmp/candidate.cross-realm")" == "$(value cross_realm_retained_variants)" ]] \
        || die 'R3dl focused report partition drifted'

    awk -F'\t' '{
        if($2!~/^(sloppy|strict)$/||$7!="unsupported-feature"||$8!="selection"||
           $9!="EngineCapability"||
           $10!~/^quickjs-oxide does not declare Test262 feature support: /||
           $10!~/(Atomics|SharedArrayBuffer)/)exit 2
    }' "$tmp/parent.activation" \
        || die 'R3dl parent activation frontier drifted'
    awk -F'\t' '{if($7!="pass"||$8!="normal"||$9!=""||$10!="")exit 2}' \
        "$tmp/candidate.activation" \
        || die 'R3dl candidate activation outcomes drifted'
    awk -F'\t' '{if($7!="pass"||$8!="normal"||$9!=""||$10!="")exit 2}' \
        "$tmp/parent.prepass" "$tmp/candidate.prepass" \
        || die 'R3dl already-admitted Atomics.pause rows drifted'
    awk -F'\t' '{
        if($7!="unsupported-feature"||$8!="selection"||$9!="EngineCapability"||
           $10!="quickjs-oxide does not declare Test262 feature support: SharedArrayBuffer, cross-realm")exit 2
    }' "$tmp/parent.cross-realm" \
        || die 'R3dl parent cross-realm diagnostics drifted'
    awk -F'\t' '{
        if($7!="unsupported-feature"||$8!="selection"||$9!="EngineCapability"||
           $10!="quickjs-oxide does not declare Test262 feature support: cross-realm")exit 2
    }' "$tmp/candidate.cross-realm" \
        || die 'R3dl candidate cross-realm retention drifted'

    check_file "$transition" "$(value transition_lines)" "$(value transition_sha256)"
    [[ "$(header "$transition" parent_commit)" == "$(value parent_commit)" \
        && "$(header "$transition" parent_profile_sha256)" == "$(value parent_profile_sha256)" \
        && "$(header "$transition" candidate_profile_sha256)" == "$(value candidate_profile_sha256)" \
        && "$(header "$transition" manifest_sha256)" == "$(value manifest_sha256)" \
        && "$(report_rows "$transition" | sha /dev/stdin)" == "$(value transition_data_sha256)" \
        && "$(transition_counts "$transition")" == \
            "changed=$(value transition_changed) outcome=$(value transition_outcome_changed) detail=$(value transition_detail_only) unchanged=$(value transition_unchanged) gains=$(value transition_pass_gains) regressions=$(value transition_pass_regressions)" ]] \
        || die 'R3dl focused transition drifted'
}

check_history_and_upstream() {
    check_file "$predecessor_baseline" "$(value predecessor_baseline_lines)" \
        "$(value predecessor_baseline_sha256)"
    [[ "$(predecessor_value candidate_profile_sha256)" == "$(value parent_profile_sha256)" \
        && "$(predecessor_value candidate_features)" == "$(value parent_features)" \
        && "$(predecessor_value candidate_features_sha256)" == "$(value parent_features_sha256)" \
        && "$(predecessor_value full_variants)" == "$(value full_variants)" \
        && "$(predecessor_value full_keys_sha256)" == "$(value full_keys_sha256)" \
        && "$(toml_test262_value repository)" == https://github.com/tc39/test262.git \
        && "$(toml_test262_value commit)" == "$(value test262)" \
        && "$(toml_test262_value patch_sha256)" == "$(value test262_patch_sha256)" \
        && "$(toml_test262_value config_sha256)" == "$(value test262_config_sha256)" \
        && "$(toml_test262_value test_count)" == "$(value test262_metadata_records)" \
        && "$(toml_test262_value metadata_records_sha256)" == "$(value test262_metadata_sha256)" \
        && "$(toml_test262_value oxide_profile)" == "$live_profile" \
        && "$(toml_test262_value oxide_profile_sha256)" == "$(value candidate_profile_sha256)" ]] \
        || die 'R3dl predecessor or upstream bridge drifted'

    # R3dl is now historical. Its immutable, checksum-bound baseline owns both
    # full-vector snapshots; never read the mutable current canonical baseline.
    [[ "$(value canonical_baseline_lines)" == 8 \
        && "$(value canonical_parent_sha256)" == \
            8c9b69ff622518433e88cedfa4735d0b9e0a02df2723dd15883ac9be74bbd01e \
        && "$(value canonical_candidate_sha256)" == \
            65b88033180df8f96d06348999b378d15ed9aa5854ba403547561a75f25701b1 \
        && "$(value candidate_full_runnable)" == 66528 \
        && "$(value candidate_full_passes)" == 66476 \
        && "$(value candidate_full_tsv_sha256)" == \
            501b64ed5c8367f33408225d956a262619163adf52baadf28f02811d14f3eae9 \
        && "$(value candidate_full_jsonl_sha256)" == \
            610e16ba65a0239556842efec7a745ba2885c72dfb3b8447c2578b8767ef7d40 ]] \
        || die 'R3dl historical canonical snapshot drifted'
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
            || die "R3dl runner failed the exact $label profile handshake"

        for rejected in "$activation" "$already_admitted" \
            "$cross_realm_retained" \
            tests/test262-atomics-wait-nonagent-bounded.txt Cargo.toml; do
            if "$runner" --suite "$suite" --config "$source_dir/test262.conf" \
                --oxide-profile "$root/$profile" --manifest "$root/$rejected" \
                --report "$tmp/rejected.tsv" --mode both \
                --timeout-ms "$(value timeout_ms)" --workers 1 --allow-failures \
                >/dev/null 2>&1; then
                die "R3dl $label profile accepted a non-R3dl manifest: $rejected"
            fi
        done
        if "$runner" --suite "$suite" --config "$source_dir/test262.conf" \
            --oxide-profile "$root/$profile" \
            --test test/built-ins/Atomics/add/descriptor.js \
            --report "$tmp/rejected.tsv" --mode both \
            --timeout-ms "$(value timeout_ms)" --workers 1 --allow-failures \
            >/dev/null 2>&1; then
            die "R3dl $label profile accepted --test"
        fi
    done
}

verify_quickjs() {
    local test_path
    local -a files=()
    [[ -x "$source_dir/run-test262" ]] \
        || "${MAKE:-make}" -C "$source_dir" run-test262 >&2
    while IFS= read -r test_path; do files+=("test262/$test_path"); done <"$manifest"
    if ! (cd -- "$source_dir" && \
        ./run-test262 -m -c test262.conf -a -T "$workers" -f "${files[@]}") \
        >"$root/$oracle_log" 2>&1; then
        tail -n 100 "$oracle_log" >&2
        die 'pinned QuickJS could not execute the R3dl manifest'
    fi
    if grep -Eq '(^|[[:space:]])FAILED($|[[:space:]])|SKIPPED FEATURE' "$oracle_log" \
        || ! grep -Fq "Average memory statistics for $(value quickjs_variants) tests:" \
            "$oracle_log"; then
        tail -n 100 "$oracle_log" >&2
        die 'pinned QuickJS no longer passes all R3dl variants'
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
        || die 'R3dl parent focused replay drifted'
    cmp -s "$candidate_report" "$candidate_replay" \
        && cmp -s "${candidate_report%.tsv}.jsonl" "${candidate_replay%.tsv}.jsonl" \
        || die 'R3dl candidate focused replay drifted'
    make_transition "$parent_replay" "$candidate_replay" "$tmp/replayed-transition.tsv"
    diff -u "$transition" "$tmp/replayed-transition.tsv"
}

run_full_report() {
    local profile=$1 output=$2
    "$runner" --suite "$suite" --config "$source_dir/test262.conf" \
        --oxide-profile "$root/$profile" --all --report "$root/$output" \
        --mode both --timeout-ms "$(value timeout_ms)" --workers "$full_workers" \
        --allow-failures >/dev/null
}

verify_full_join() {
    local counts expected parent_json=${parent_full%.tsv}.jsonl
    local candidate_json=${candidate_full%.tsv}.jsonl
    rows_for_paths "$manifest" "$parent_full" >"$tmp/parent.scope"
    rows_for_paths "$manifest" "$candidate_full" >"$tmp/candidate.scope"
    rows_without_paths "$manifest" "$parent_full" >"$tmp/parent.outside"
    rows_without_paths "$manifest" "$candidate_full" >"$tmp/candidate.outside"
    json_rows_for_paths "$manifest" "$parent_json" >"$tmp/parent.scope.json"
    json_rows_for_paths "$manifest" "$candidate_json" >"$tmp/candidate.scope.json"
    json_rows_without_paths "$manifest" "$parent_json" >"$tmp/parent.outside.json"
    json_rows_without_paths "$manifest" "$candidate_json" >"$tmp/candidate.outside.json"
    report_rows "$parent_report" >"$tmp/focused.parent"
    report_rows "$candidate_report" >"$tmp/focused.candidate"
    json_result_rows "${parent_report%.tsv}.jsonl" >"$tmp/focused.parent.json"
    json_result_rows "${candidate_report%.tsv}.jsonl" >"$tmp/focused.candidate.json"
    [[ "$(lines "$tmp/parent.scope")" == "$(value full_scope_rows)" \
        && "$(lines "$tmp/candidate.scope")" == "$(value full_scope_rows)" \
        && "$(lines "$tmp/parent.outside")" == "$(value full_outside_rows)" \
        && "$(lines "$tmp/candidate.outside")" == "$(value full_outside_rows)" ]] \
        || die 'R3dl full partition row counts drifted'
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
    ' "$parent_full" "$candidate_full") || die 'R3dl exact full join failed'
    expected="changed=$(value full_changed) outcome=$(value full_outcome_changed) detail=$(value full_detail_only) unchanged=$(value full_unchanged) regressions=$(value full_pass_regressions)"
    [[ "$counts" == "$expected" ]] || die "R3dl full transition drifted: $counts"
}

replay_full() {
    full_receipt_is_blessed \
        || die 'R3dl full hashes are explicitly PENDING_FULL; run and bless the full receipts first'
    if [[ "$reuse_full_reports" == false \
        || ! -f "$parent_full" || ! -f "${parent_full%.tsv}.jsonl" \
        || "$(sha "$parent_full")" != "$(value parent_full_tsv_sha256)" \
        || "$(sha "${parent_full%.tsv}.jsonl")" != "$(value parent_full_jsonl_sha256)" ]]; then
        run_full_report "$parent_profile" "$parent_full"
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
        && "$(report_count unsupported-feature "$candidate_full")" == "$(value candidate_full_unsupported_feature)" ]] \
        || die 'R3dl full outcome counts drifted'
    verify_full_join
}

bridge_r3dm_successor() {
    [[ "$(toml_test262_value oxide_profile_sha256)" != \
        "$(value candidate_profile_sha256)" ]] || return 0

    check_file "$baseline" "$baseline_lines" "$baseline_sha"
    check_file "$successor_baseline" "$successor_lines" "$successor_sha"
    [[ -x "$successor_gate" \
        && "$(successor_value milestone_kind)" == global-profile-admission \
        && "$(successor_value quickjs)" == "$(value quickjs)" \
        && "$(successor_value test262)" == "$(value test262)" \
        && "$(successor_value test262_patch_sha256)" == \
            "$(value test262_patch_sha256)" \
        && "$(successor_value test262_config_sha256)" == \
            "$(value test262_config_sha256)" \
        && "$(successor_value test262_metadata_sha256)" == \
            "$(value test262_metadata_sha256)" \
        && "$(successor_value schema)" == "$(value schema)" \
        && "$(successor_value mode)" == "$(value mode)" \
        && "$(successor_value timeout_ms)" == "$(value timeout_ms)" \
        && "$(successor_value parent_commit)" == \
            fa64fa1da55ba130954e2279f1bb8c05bf71fe08 \
        && "$(successor_value predecessor_baseline)" == "$baseline" \
        && "$(successor_value predecessor_baseline_lines)" == "$baseline_lines" \
        && "$(successor_value predecessor_baseline_sha256)" == "$baseline_sha" \
        && "$(successor_value parent_profile_sha256)" == \
            "$(value candidate_profile_sha256)" \
        && "$(successor_value full_variants)" == "$(value full_variants)" \
        && "$(successor_value full_keys_sha256)" == \
            "$(value full_keys_sha256)" \
        && "$(successor_value parent_full_runnable)" == \
            "$(value candidate_full_runnable)" \
        && "$(successor_value parent_full_passes)" == \
            "$(value candidate_full_passes)" \
        && "$(successor_value parent_full_unsupported_feature)" == \
            "$(value candidate_full_unsupported_feature)" \
        && "$(successor_value parent_full_tsv_sha256)" == \
            "$(value candidate_full_tsv_sha256)" \
        && "$(successor_value parent_full_jsonl_sha256)" == \
            "$(value candidate_full_jsonl_sha256)" \
        && "$(successor_value parent_full_summary)" == \
            "$(value candidate_full_summary)" \
        && "$(successor_value manifest_paths)" == 59 \
        && "$(successor_value manifest_variants)" == 118 \
        && "$(successor_value activation_variants)" == 2 \
        && "$(successor_value retained_variants)" == 116 \
        && "$(( $(successor_value candidate_full_runnable) - \
            $(successor_value parent_full_runnable) ))" == 2 \
        && "$(( $(successor_value candidate_full_passes) - \
            $(successor_value parent_full_passes) ))" == 2 \
        && "$(( $(successor_value parent_full_unsupported_host_agent) - \
            $(successor_value candidate_full_unsupported_host_agent) ))" == 2 \
        && "$(successor_value transition_outcome_changed)" == 2 \
        && "$(successor_value transition_detail_only)" == 0 \
        && "$(successor_value transition_unchanged)" == 116 \
        && "$(successor_value transition_pass_regressions)" == 0 \
        && "$(successor_value full_changed)" == 2 \
        && "$(successor_value full_outcome_changed)" == 2 \
        && "$(successor_value full_detail_only)" == 0 \
        && "$(successor_value full_unchanged)" == 102035 \
        && "$(successor_value full_pass_regressions)" == 0 ]] \
        || die 'R3dl successor bridge to R3dm drifted'

    case $mode in
        check) "$successor_gate" --check ;;
        focused) "$successor_gate" ;;
        full) TEST262_REUSE_FULL_REPORTS="$reuse_full_reports" \
            "$successor_gate" --full ;;
    esac
    exit 0
}

cd -- "$root"
bridge_r3dm_successor
tmp=$(mktemp -d "${TMPDIR:-/tmp}/quickjs-oxide-r3dl.XXXXXX")
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
    printf 'R3dl SharedArrayBuffer/Atomics full: 866 new passes, 8 retained cross-realm diagnostics, zero regressions\n'
else
    printf 'R3dl SharedArrayBuffer/Atomics focused: 866/866 activated pass, 12 parent passes retained, 8 cross-realm diagnostics retained\n'
fi
