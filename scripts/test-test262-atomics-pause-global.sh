#!/usr/bin/env bash
# Reproduce the R3df global Atomics.pause Test262 admission.

set -euo pipefail
export LC_ALL=C
export TZ=America/Los_Angeles

script_dir=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd)
root=$(CDPATH='' cd -- "$script_dir/.." && pwd)
baseline=tests/test262-atomics-pause-global-baseline.txt
predecessor_baseline=tests/test262-atomics-non-shared-core-baseline.txt
canonical_baseline=tests/test262-full-baseline.txt
upstream=compat/upstream.toml
live_profile=compat/test262-oxide.conf
parent_profile=tests/test262-atomics-pause-global-parent.conf
candidate_profile=tests/test262-atomics-pause-global-candidate.conf
added_features=tests/test262-atomics-pause-global-added-features.txt
universe_ledger=tests/test262-atomics-universe.tsv
manifest=tests/test262-atomics-pause-global.txt
parent_report=tests/test262-atomics-pause-global-parent.tsv
candidate_report=tests/test262-atomics-pause-global-candidate.tsv
transition=tests/test262-atomics-pause-global-transitions.tsv
parent_replay=target/test262-atomics-pause-global-parent-replay.tsv
candidate_replay=target/test262-atomics-pause-global-candidate-replay.tsv
parent_full=target/test262-atomics-non-shared-core-full.tsv
candidate_full=target/test262-atomics-pause-global-candidate-full.tsv
oracle_log=target/test262-atomics-pause-global-quickjs.log
workers=${TEST262_WORKERS:-8}
full_workers=${TEST262_FULL_WORKERS:-2}
reuse_full_reports=${TEST262_REUSE_FULL_REPORTS:-false}
runner_override=${TEST262_RUNNER:-}

baseline_lines=107
baseline_sha=4fc276dba2ebf606a641562a4921122e7bb2215fa3ddcbe6cb4896ca6ed6779c

usage() {
    printf 'usage: %s [--check|--full]\n' "${0##*/}"
    printf '  --check  authenticate frozen profiles, focused receipts, metadata, and QuickJS\n'
    printf '  default  additionally replay the exact 6-path / 12-variant transition\n'
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
        echo '# Exhaustive R3df Atomics.pause global admission transition.'
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
        || die 'live profile is not byte-identical to the R3df candidate'

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
        || die 'R3df profile inventory drifted'
    diff -u "$added_features" "$tmp/added.features"
    [[ -z "$(comm -23 "$tmp/parent.features" "$tmp/candidate.features")" ]] \
        || die 'R3df candidate removed a parent feature'
    diff -u "$tmp/parent.audited-negative-tests" "$tmp/candidate.audited-negative-tests"
    diff -u "$tmp/parent.execution" "$tmp/candidate.execution"
}

check_manifest_and_ledger() {
    check_file "$added_features" "$(value added_features_count)" \
        "$(value added_features_sha256)"
    check_file "$manifest" "$(value manifest_paths)" "$(value manifest_sha256)"
    check_file "$universe_ledger" "$(value universe_ledger_lines)" \
        "$(value universe_ledger_sha256)"
    sort -c "$manifest" || die 'R3df manifest is not bytewise sorted'
    [[ -z "$(uniq -d "$manifest")" ]] || die 'R3df manifest contains duplicates'

    awk -F'\t' '
        function has(list,value){return index("," list ",","," value ",")!=0}
        NR==1{next}
        has($5,"Atomics.pause"){
            if($2!="non-shared-no-sab-tag"||$4!="")bad=1
            print
        }
        END{if(bad)exit 2}
    ' "$universe_ledger" >"$tmp/pause.ledger"
    cut -f1 "$tmp/pause.ledger" >"$tmp/pause.paths"
    cut -f6 "$tmp/pause.ledger" | paste "$tmp/pause.paths" - >"$tmp/pause.sources"
    awk -F'\t' '{n=split($5,a,",");for(i=1;i<=n;i++)if(a[i]!="")print a[i]}' \
        "$tmp/pause.ledger" | sort -u >"$tmp/pause.features"
    awk -F'\t' '$3!=""{n=split($3,a,",");for(i=1;i<=n;i++)if(a[i]!="")print a[i]}' \
        "$tmp/pause.ledger" | sort -u >"$tmp/pause.includes"
    [[ "$(sha "$tmp/pause.ledger")" == "$(value manifest_ledger_rows_sha256)" \
        && "$(sha "$tmp/pause.sources")" == "$(value manifest_source_projection_sha256)" \
        && "$(lines "$tmp/pause.features")" == "$(value manifest_metadata_features)" \
        && "$(sha "$tmp/pause.features")" == "$(value manifest_metadata_features_sha256)" \
        && "$(lines "$tmp/pause.includes")" == "$(value manifest_metadata_includes)" \
        && "$(sha "$tmp/pause.includes")" == "$(value manifest_metadata_includes_sha256)" ]] \
        || die 'R3df audited ledger partition drifted'
    diff -u "$manifest" "$tmp/pause.paths"

    local path expected actual
    while IFS=$'\t' read -r path expected; do
        actual=$(sha "$suite/$path")
        [[ "$actual" == "$expected" ]] || die "R3df source drifted: $path"
        ! grep -Eq 'SharedArrayBuffer|\$262' "$suite/$path" \
            || die "R3df source gained a shared-memory or host dependency: $path"
    done <"$tmp/pause.sources"
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
        && "$(report_count unsupported-feature "$parent_report")" == "$(value parent_unsupported_feature)" \
        && "$(report_runnable "$candidate_report")" == "$(value candidate_runnable)" \
        && "$(report_count pass "$candidate_report")" == "$(value candidate_passes)" ]] \
        || die 'R3df focused outcome counts drifted'
    awk -F'\t' '!/^#/&&!($1=="path"&&$2=="variant"){
        if($2!~/^(sloppy|strict)$/||$7!="unsupported-feature"||$8!="selection"||
           $9!="EngineCapability"||$10!="quickjs-oxide does not declare Test262 feature support: Atomics.pause")exit 2
    }' "$parent_report" || die 'R3df parent selection frontier drifted'
    awk -F'\t' '!/^#/&&!($1=="path"&&$2=="variant"){
        if($2!~/^(sloppy|strict)$/||$7!="pass"||$8!="normal"||$9!=""||$10!="")exit 2
    }' "$candidate_report" || die 'R3df candidate semantics drifted'

    check_file "$transition" "$(value transition_lines)" "$(value transition_sha256)"
    [[ "$(header "$transition" parent_commit)" == "$(value parent_commit)" \
        && "$(header "$transition" parent_profile_sha256)" == "$(value parent_profile_sha256)" \
        && "$(header "$transition" candidate_profile_sha256)" == "$(value candidate_profile_sha256)" \
        && "$(header "$transition" manifest_sha256)" == "$(value manifest_sha256)" \
        && "$(report_rows "$transition" | sha /dev/stdin)" == "$(value transition_data_sha256)" \
        && "$(transition_counts "$transition")" == \
            "changed=$(value transition_changed) outcome=$(value transition_outcome_changed) detail=$(value transition_detail_only) unchanged=$(value transition_unchanged) gains=$(value transition_pass_gains) regressions=$(value transition_pass_regressions)" ]] \
        || die 'R3df focused transition drifted'
}

check_history_and_upstream() {
    check_file "$predecessor_baseline" "$(value predecessor_baseline_lines)" \
        "$(value predecessor_baseline_sha256)"
    [[ "$(predecessor_value global_profile_sha256)" == "$(value parent_profile_sha256)" \
        && "$(predecessor_value universe_pause_paths)" == "$(value manifest_paths)" \
        && "$(predecessor_value universe_pause_variants)" == "$(value manifest_variants)" \
        && "$(predecessor_value universe_pause_paths_sha256)" == "$(value manifest_sha256)" \
        && "$(predecessor_value full_variants)" == "$(value full_variants)" \
        && "$(predecessor_value full_keys_sha256)" == "$(value full_keys_sha256)" \
        && "$(predecessor_value full_runnable)" == "$(value parent_full_runnable)" \
        && "$(predecessor_value full_passes)" == "$(value parent_full_passes)" \
        && "$(predecessor_value full_tsv_sha256)" == "$(value parent_full_tsv_sha256)" \
        && "$(predecessor_value full_jsonl_sha256)" == "$(value parent_full_jsonl_sha256)" \
        && "$(predecessor_value full_summary)" == "$(value parent_full_summary)" \
        && "$(toml_test262_value repository)" == https://github.com/tc39/test262.git \
        && "$(toml_test262_value commit)" == "$(value test262)" \
        && "$(toml_test262_value patch_sha256)" == "$(value test262_patch_sha256)" \
        && "$(toml_test262_value config_sha256)" == "$(value test262_config_sha256)" \
        && "$(toml_test262_value test_count)" == "$(value test262_metadata_records)" \
        && "$(toml_test262_value metadata_records_sha256)" == "$(value test262_metadata_sha256)" \
        && "$(toml_test262_value oxide_profile)" == "$live_profile" \
        && "$(toml_test262_value oxide_profile_sha256)" == "$(value candidate_profile_sha256)" ]] \
        || die 'R3df predecessor or upstream bridge drifted'

    check_file "$canonical_baseline" "$(value canonical_baseline_lines)" \
        "$(value canonical_baseline_sha256)"
    if full_receipt_is_blessed; then
        [[ "$(canonical_value schema)" == "$(value schema)" \
            && "$(canonical_value timeout_ms)" == "$(value timeout_ms)" \
            && "$(canonical_value variants)" == "$(value full_variants)" \
            && "$(canonical_value runnable)" == "$(value candidate_full_runnable)" \
            && "$(canonical_value passes)" == "$(value candidate_full_passes)" \
            && "$(canonical_value tsv_sha256)" == "$(value candidate_full_tsv_sha256)" \
            && "$(canonical_value jsonl_sha256)" == "$(value candidate_full_jsonl_sha256)" \
            && "$(canonical_value summary)" == "$(value candidate_full_summary)" ]] \
            || die 'R3df candidate is not the canonical full vector'
    else
        [[ "$(canonical_value runnable)" == "$(value parent_full_runnable)" \
            && "$(canonical_value passes)" == "$(value parent_full_passes)" \
            && "$(canonical_value tsv_sha256)" == "$(value parent_full_tsv_sha256)" \
            && "$(canonical_value jsonl_sha256)" == "$(value parent_full_jsonl_sha256)" \
            && "$(canonical_value summary)" == "$(value parent_full_summary)" ]] \
            || die 'unblessed R3df gate is not based on the R3de canonical vector'
    fi
}

check_metadata() {
    "$runner" --suite "$suite" --validate-metadata "$tmp/metadata.bin" >/dev/null
    [[ "$(lines "$tmp/metadata.bin")" == "$(value test262_metadata_records)" \
        && "$(sha "$tmp/metadata.bin")" == "$(value test262_metadata_sha256)" ]] \
        || die 'pinned Test262 metadata drifted'
    tr '\0' '\t' <"$tmp/metadata.bin" >"$tmp/metadata.tsv"
    awk -F'\t' '
        function has(list,value){return index("," list ",","," value ",")!=0}
        has($4,"Atomics.pause"){
            if($3!=""||$5!=""||$6!="")bad=1
            print $1
            n=split($4,a,",");for(i=1;i<=n;i++)if(a[i]!="")features[a[i]]=1
        }
        END{
            if(bad)exit 2
            for(feature in features)print feature > features_output
        }
    ' features_output="$tmp/metadata.features.unsorted" "$tmp/metadata.tsv" \
        | sort -u >"$tmp/metadata.paths" \
        || die 'Atomics.pause metadata envelope gained unsupported flags or negatives'
    sort -u "$tmp/metadata.features.unsorted" >"$tmp/metadata.features"
    diff -u "$manifest" "$tmp/metadata.paths"
    [[ "$(lines "$tmp/metadata.features")" == "$(value manifest_metadata_features)" \
        && "$(sha "$tmp/metadata.features")" == "$(value manifest_metadata_features_sha256)" ]] \
        || die 'Atomics.pause metadata feature closure drifted'
}

verify_fail_closed_selection() {
    local common=(--suite "$suite" --config "$source_dir/test262.conf"
        --oxide-profile "$root/$candidate_profile" --report "$tmp/rejected.tsv"
        --mode both --timeout-ms "$(value timeout_ms)" --workers 1 --allow-failures)
    "$runner" --suite "$suite" --config "$source_dir/test262.conf" \
        --oxide-profile "$root/$candidate_profile" --manifest "$root/$manifest" \
        --report "$tmp/positive.tsv" --mode both \
        --timeout-ms "$(value timeout_ms)" --workers 1 >/dev/null
    cmp -s "$candidate_report" "$tmp/positive.tsv" \
        && cmp -s "${candidate_report%.tsv}.jsonl" "$tmp/positive.jsonl" \
        || die 'R3df runner failed the exact positive profile handshake'
    if "$runner" "${common[@]}" --manifest "$root/tests/test262-atomics-shared-deferred.txt" \
        >/dev/null 2>&1; then
        die 'R3df candidate accepted the shared-memory deferred manifest'
    fi
    if "$runner" "${common[@]}" --manifest "$root/tests/test262-atomics-non-shared-core.txt" \
        >/dev/null 2>&1; then
        die 'R3df candidate accepted the broader R3de manifest'
    fi
    if "$runner" "${common[@]}" --test test/built-ins/Atomics/pause/descriptor.js \
        >/dev/null 2>&1; then
        die 'R3df candidate accepted --test'
    fi
}

verify_quickjs() {
    local path
    local -a files=()
    [[ -x "$source_dir/run-test262" ]] \
        || "${MAKE:-make}" -C "$source_dir" run-test262 >&2
    while IFS= read -r path; do files+=("test262/$path"); done <"$manifest"
    if ! (cd -- "$source_dir" && \
        ./run-test262 -m -c test262.conf -a -T "$workers" -f "${files[@]}") \
        >"$root/$oracle_log" 2>&1; then
        tail -n 100 "$oracle_log" >&2
        die 'pinned QuickJS could not execute the R3df manifest'
    fi
    if grep -Eq '(^|[[:space:]])FAILED($|[[:space:]])|SKIPPED FEATURE' "$oracle_log" \
        || ! grep -Fq "Average memory statistics for $(value quickjs_variants) tests:" \
            "$oracle_log"; then
        tail -n 100 "$oracle_log" >&2
        die 'pinned QuickJS no longer passes all R3df variants'
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
        || die 'R3df parent focused replay drifted'
    cmp -s "$candidate_report" "$candidate_replay" \
        && cmp -s "${candidate_report%.tsv}.jsonl" "${candidate_replay%.tsv}.jsonl" \
        || die 'R3df candidate focused replay drifted'
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
    rows_for_paths "$manifest" "$parent_full" >"$tmp/parent.universe"
    rows_for_paths "$manifest" "$candidate_full" >"$tmp/candidate.universe"
    rows_without_paths "$manifest" "$parent_full" >"$tmp/parent.non-universe"
    rows_without_paths "$manifest" "$candidate_full" >"$tmp/candidate.non-universe"
    json_rows_for_paths "$manifest" "$parent_json" >"$tmp/parent.universe.json"
    json_rows_for_paths "$manifest" "$candidate_json" >"$tmp/candidate.universe.json"
    json_rows_without_paths "$manifest" "$parent_json" >"$tmp/parent.non-universe.json"
    json_rows_without_paths "$manifest" "$candidate_json" >"$tmp/candidate.non-universe.json"
    report_rows "$parent_report" >"$tmp/focused.parent"
    report_rows "$candidate_report" >"$tmp/focused.candidate"
    json_result_rows "${parent_report%.tsv}.jsonl" >"$tmp/focused.parent.json"
    json_result_rows "${candidate_report%.tsv}.jsonl" >"$tmp/focused.candidate.json"
    [[ "$(lines "$tmp/parent.universe")" == "$(value full_universe_rows)" \
        && "$(lines "$tmp/candidate.universe")" == "$(value full_universe_rows)" \
        && "$(lines "$tmp/parent.non-universe")" == "$(value full_non_universe_rows)" \
        && "$(lines "$tmp/candidate.non-universe")" == "$(value full_non_universe_rows)" ]] \
        || die 'R3df full partition row counts drifted'
    diff -u "$tmp/focused.parent" "$tmp/parent.universe"
    diff -u "$tmp/focused.candidate" "$tmp/candidate.universe"
    diff -u "$tmp/focused.parent.json" "$tmp/parent.universe.json"
    diff -u "$tmp/focused.candidate.json" "$tmp/candidate.universe.json"
    diff -u "$tmp/parent.non-universe" "$tmp/candidate.non-universe"
    diff -u "$tmp/parent.non-universe.json" "$tmp/candidate.non-universe.json"

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
    ' "$parent_full" "$candidate_full") || die 'R3df exact full join failed'
    expected="changed=$(value full_changed) outcome=$(value full_outcome_changed) detail=$(value full_detail_only) unchanged=$(value full_unchanged) regressions=$(value full_pass_regressions)"
    [[ "$counts" == "$expected" ]] || die "R3df full transition drifted: $counts"
}

replay_full() {
    full_receipt_is_blessed \
        || die 'R3df full hashes are explicitly PENDING_FULL; run and bless the full receipts first'
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
        || die 'R3df full outcome counts drifted'
    verify_full_join
}

cd -- "$root"
tmp=$(mktemp -d "${TMPDIR:-/tmp}/quickjs-oxide-r3df.XXXXXX")
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
check_manifest_and_ledger
check_receipts
check_history_and_upstream
check_metadata
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
    printf 'R3df Atomics.pause full: 12 new passes, 102025 unchanged rows, zero regressions\n'
else
    printf 'R3df Atomics.pause focused: parent=12 unsupported candidate=12/12 pass\n'
fi
