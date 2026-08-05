#!/usr/bin/env bash
# Reproduce the R3da generator yield-star stack-budget milestone.

set -euo pipefail
export LC_ALL=C
export TZ=America/Los_Angeles

script_dir=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd)
root=$(CDPATH='' cd -- "$script_dir/.." && pwd)
baseline=tests/test262-generator-yield-star-stack-budget-baseline.txt
predecessor_baseline=tests/test262-class-field-await-baseline.txt
canonical_baseline=tests/test262-full-baseline.txt
upstream=compat/upstream.toml
profile=compat/test262-oxide.conf
manifest=tests/test262-generator-yield-star-stack-budget.txt
parent_report=tests/test262-generator-yield-star-stack-budget-parent.tsv
candidate_report=tests/test262-generator-yield-star-stack-budget-candidate.tsv
transition=tests/test262-generator-yield-star-stack-budget-transitions.tsv
parent_replay=target/test262-generator-yield-star-stack-budget-parent-replay.tsv
candidate_replay=target/test262-generator-yield-star-stack-budget-candidate-replay.tsv
preferred_parent_full=${TEST262_GENERATOR_YIELD_STAR_STACK_BUDGET_PARENT_FULL:-target/test262-class-field-await-full.tsv}
generated_parent_full=target/test262-generator-yield-star-stack-budget-parent-full.tsv
candidate_full=target/test262-generator-yield-star-stack-budget-full.tsv
oracle_log=target/test262-generator-yield-star-stack-budget-quickjs.log
workers=${TEST262_WORKERS:-2}
full_workers=${TEST262_FULL_WORKERS:-2}
reuse_full_reports=${TEST262_REUSE_FULL_REPORTS:-false}
runner_override=${TEST262_RUNNER:-}
parent_runner_override=${TEST262_GENERATOR_YIELD_STAR_STACK_BUDGET_PARENT_RUNNER:-}

baseline_lines=84
baseline_sha=c0d9ddd3bec82f65b4e1b6cfcaff807d0744c3e7215e1bb122a30ef3dd47d760
predecessor_lines=84
predecessor_sha=b7e22ee00fad7c0e4fe25736d204d7627393ca806235514ee03f8981d4d06663
canonical_lines=8
canonical_sha=4a3f8eab0b4e882108b95c54743b9d4a6d6d7411d4d6de83915c24f2b9fbd899

usage() {
    printf 'usage: %s [--check|--full]\n' "${0##*/}"
    printf '  --check  verify frozen receipts, runner inputs, metadata, and QuickJS\n'
    printf '  --full   additionally replay and exact-join all 102037 variants\n'
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

verify_report() {
    local report=$1 label=$2 json=${1%.tsv}.jsonl
    [[ -f "$report" && -f "$json" \
        && "$(header "$report" quickjs)" == "$(value quickjs)" \
        && "$(header "$report" test262)" == "$(value test262)" \
        && "$(header "$report" test262_patch_sha256)" == "$(value test262_patch_sha256)" \
        && "$(header "$report" test262_config_sha256)" == "$(value test262_config_sha256)" \
        && "$(header "$report" test262_metadata_sha256)" == "$(value test262_metadata_sha256)" \
        && "$(header "$report" oxide_profile_sha256)" == "$(value profile_sha256)" \
        && "$(header "$report" profile)" == "$(value schema)" \
        && "$(header "$report" mode)" == "$(value mode)" \
        && "$(report_rows "$report" | wc -l | tr -d '[:space:]')" == "$(value manifest_variants)" \
        && "$(report_keys "$report" | sha /dev/stdin)" == "$(value manifest_keys_sha256)" \
        && "$(report_rows "$report" | sha /dev/stdin)" == "$(value "${label}_rows_sha256")" \
        && "$(report_summary "$report")" == "$(value "${label}_summary")" \
        && "$(computed_summary "$report")" == "$(value "${label}_summary")" \
        && "$(sha "$report")" == "$(value "${label}_tsv_sha256")" \
        && "$(sha "$json")" == "$(value "${label}_jsonl_sha256")" ]] \
        || die "classified report drifted: $report"
}

verify_full_report() {
    local report=$1 label=$2 json=${1%.tsv}.jsonl
    local expected_summary
    expected_summary=$(value "${label}_summary")
    [[ -f "$report" && -f "$json" \
        && "$(header "$report" quickjs)" == "$(value quickjs)" \
        && "$(header "$report" test262)" == "$(value test262)" \
        && "$(header "$report" test262_patch_sha256)" == "$(value test262_patch_sha256)" \
        && "$(header "$report" test262_config_sha256)" == "$(value test262_config_sha256)" \
        && "$(header "$report" test262_metadata_sha256)" == "$(value test262_metadata_sha256)" \
        && "$(header "$report" oxide_profile_sha256)" == "$(value profile_sha256)" \
        && "$(header "$report" profile)" == "$(value schema)" \
        && "$(header "$report" mode)" == "$(value mode)" \
        && "$(report_rows "$report" | wc -l | tr -d '[:space:]')" == "$(value full_variants)" \
        && "$(report_keys "$report" | sha /dev/stdin)" == "$(value full_keys_sha256)" \
        && "$(report_summary "$report")" == "$expected_summary" \
        && "$(computed_summary "$report")" == "$expected_summary" ]] \
        || die "full classified report drifted: $report"
    [[ "$(sha "$report")" == "$(value "${label}_tsv_sha256")" \
        && "$(sha "$json")" == "$(value "${label}_jsonl_sha256")" ]] \
        || die "full classified receipt drifted: $report"
}

make_transition() {
    local before=$1 after=$2 output=$3
    {
        echo '# Exhaustive R3da generator yield-star stack-budget transition.'
        echo "# parent_commit=$(value parent_commit)"
        echo "# oxide_profile_sha256=$(value profile_sha256)"
        echo "# manifest_sha256=$(value manifest_sha256)"
        printf 'path\tvariant\tflags\tfeatures\texpected_phase\texpected_type\tbefore_outcome\tbefore_actual_phase\tbefore_actual_type\tbefore_detail\tafter_outcome\tafter_actual_phase\tafter_actual_type\tafter_detail\n'
        awk -F'\t' 'BEGIN{OFS="\t"}
            NR==FNR{if(!/^#/&&!($1=="path"&&$2=="variant"))old[$1 FS $2]=$0;next}
            !/^#/&&!($1=="path"&&$2=="variant"){
                key=$1 FS $2;if(!(key in old))exit 2;split(old[key],a,FS)
                print $1,$2,$3,$4,$5,$6,a[7],a[8],a[9],a[10],$7,$8,$9,$10
                seen[key]=1
            }
            END{for(key in old)if(!(key in seen))exit 3}
        ' "$before" "$after"
    } >"$output"
}

transition_counts() {
    awk -F'\t' '!/^#/&&!($1=="path"&&$2=="variant"){
        different=0;for(i=7;i<=10;i++)if($i!=$(i+4))different=1
        if($7=="pass"&&$11!="pass")regress++
        if(different){changed++;if($7!=$11)outcome++;else detail++}else unchanged++
    } END{printf "changed=%d outcome=%d detail=%d unchanged=%d regressions=%d",changed,outcome,detail,unchanged,regress}' "$1"
}

check_profile() {
    check_file "$profile" "$(value profile_lines)" "$(value profile_sha256)"
    for section in features audited-negative-tests execution; do
        profile_section "$section" "$profile" >"$tmp/profile.$section"
    done
    [[ "$(lines "$tmp/profile.features")" == "$(value profile_features)" \
        && "$(sha "$tmp/profile.features")" == "$(value profile_features_sha256)" \
        && "$(lines "$tmp/profile.audited-negative-tests")" == "$(value profile_audited_negative_tests)" \
        && "$(sha "$tmp/profile.audited-negative-tests")" == "$(value profile_audited_negative_tests_sha256)" \
        && "$(lines "$tmp/profile.execution")" == "$(value profile_execution_entries)" \
        && "$(sha "$tmp/profile.execution")" == "$(value profile_execution_sha256)" ]] \
        || die 'R3da runner profile inventory drifted'
}

verify_parent_full_source() {
    local parent_json=${preferred_parent_full%.tsv}.jsonl
    # A fresh checkout does not carry target/ receipts. When the predecessor
    # full report is present, authenticate the focused extraction here; full
    # mode otherwise reconstructs and verifies it from the frozen focused
    # parent after producing the candidate report.
    if [[ ! -e "$preferred_parent_full" && ! -e "$parent_json" ]]; then
        return 0
    fi
    [[ -f "$preferred_parent_full" && -f "$parent_json" ]] \
        || die 'R3da predecessor full receipt is incomplete'
    verify_full_report "$preferred_parent_full" parent_full
    rows_for_paths "$manifest" "$preferred_parent_full" >"$tmp/parent.cohort"
    rows_without_paths "$manifest" "$preferred_parent_full" >"$tmp/parent.non-cohort"
    report_rows "$parent_report" >"$tmp/focused.parent"
    json_rows_for_paths "$manifest" "$parent_json" >"$tmp/parent.cohort.json"
    json_rows_without_paths "$manifest" "$parent_json" >"$tmp/parent.non-cohort.json"
    awk '/^\{"kind":"result"/' "${parent_report%.tsv}.jsonl" >"$tmp/focused.parent.json"
    [[ "$(lines "$tmp/parent.cohort")" == "$(value full_cohort_rows)" \
        && "$(sha "$tmp/parent.cohort")" == "$(value full_parent_cohort_rows_sha256)" \
        && "$(sha "$tmp/parent.cohort.json")" == "$(value full_parent_cohort_json_rows_sha256)" \
        && "$(lines "$tmp/parent.non-cohort")" == "$(value full_non_cohort_rows)" \
        && "$(sha "$tmp/parent.non-cohort")" == "$(value full_non_cohort_rows_sha256)" \
        && "$(sha "$tmp/parent.non-cohort.json")" == "$(value full_non_cohort_json_rows_sha256)" ]] \
        || die 'R3da parent full partition drifted'
    diff -u "$tmp/focused.parent" "$tmp/parent.cohort"
    diff -u "$tmp/focused.parent.json" "$tmp/parent.cohort.json"
}

check_static_inputs() {
    check_file "$baseline" "$baseline_lines" "$baseline_sha"
    check_file "$predecessor_baseline" "$predecessor_lines" "$predecessor_sha"
    check_file "$canonical_baseline" "$canonical_lines" "$canonical_sha"
    check_file "$manifest" "$(value manifest_paths)" "$(value manifest_sha256)"
    sort -c "$manifest" || die 'R3da manifest is not bytewise sorted'
    [[ -z "$(uniq -d "$manifest")" ]] || die 'R3da manifest contains duplicates'
    check_file "$parent_report" "$(value parent_focused_lines)" "$(value parent_focused_tsv_sha256)"
    check_file "${parent_report%.tsv}.jsonl" "$(value parent_focused_jsonl_lines)" "$(value parent_focused_jsonl_sha256)"
    check_file "$candidate_report" "$(value candidate_focused_lines)" "$(value candidate_focused_tsv_sha256)"
    check_file "${candidate_report%.tsv}.jsonl" "$(value candidate_focused_jsonl_lines)" "$(value candidate_focused_jsonl_sha256)"
    check_file "$transition" "$(value transition_lines)" "$(value transition_sha256)"
    verify_report "$parent_report" parent_focused
    verify_report "$candidate_report" candidate_focused
    check_profile

    [[ "$(header "$transition" parent_commit)" == "$(value parent_commit)" \
        && "$(header "$transition" oxide_profile_sha256)" == "$(value profile_sha256)" \
        && "$(header "$transition" manifest_sha256)" == "$(value manifest_sha256)" \
        && "$(report_rows "$transition" | sha /dev/stdin)" == "$(value transition_data_sha256)" \
        && "$(transition_counts "$transition")" == "changed=2 outcome=2 detail=0 unchanged=0 regressions=0" \
        && "$(toml_test262_value repository)" == https://github.com/tc39/test262.git \
        && "$(toml_test262_value commit)" == "$(value test262)" \
        && "$(toml_test262_value patch_sha256)" == "$(value test262_patch_sha256)" \
        && "$(toml_test262_value config_sha256)" == "$(value test262_config_sha256)" \
        && "$(toml_test262_value test_count)" == "$(value test262_metadata_records)" \
        && "$(toml_test262_value metadata_records_sha256)" == "$(value test262_metadata_sha256)" \
        && "$(toml_test262_value oxide_profile)" == "$profile" \
        && "$(toml_test262_value oxide_profile_sha256)" == "$(value profile_sha256)" ]] \
        || die 'R3da transition or runner input binding drifted'

    [[ "$(predecessor_value profile_sha256)" == "$(value profile_sha256)" \
        && "$(predecessor_value candidate_full_runnable)" == "$(value parent_full_runnable)" \
        && "$(predecessor_value candidate_full_passes)" == "$(value parent_full_passes)" \
        && "$(predecessor_value candidate_full_tsv_sha256)" == "$(value parent_full_tsv_sha256)" \
        && "$(predecessor_value candidate_full_jsonl_sha256)" == "$(value parent_full_jsonl_sha256)" \
        && "$(predecessor_value candidate_full_summary)" == "$(value parent_full_summary)" \
        && "$(canonical_value schema)" == "$(value schema)" \
        && "$(canonical_value timeout_ms)" == "$(value timeout_ms)" \
        && "$(canonical_value variants)" == "$(value full_variants)" \
        && "$(value candidate_full_runnable)" == "$(value parent_full_runnable)" \
        && "$(( $(value candidate_full_passes) - $(value parent_full_passes) ))" == 2 \
        && "$(value candidate_full_fail_parse)" == "$(value parent_full_fail_parse)" \
        && "$(( $(value parent_full_fail_runtime) - $(value candidate_full_fail_runtime) ))" == 2 \
        && "$(value full_changed)" == 2 \
        && "$(value full_outcome_changed)" == 2 \
        && "$(value full_detail_only)" == 0 \
        && "$(value full_unchanged)" == 102035 \
        && "$(value full_pass_regressions)" == 0 ]] \
        || die 'R3da predecessor or full-vector anchors drifted'

    [[ "$(canonical_value runnable)" == "$(value candidate_full_runnable)" \
        && "$(canonical_value passes)" == "$(value candidate_full_passes)" \
        && "$(canonical_value tsv_sha256)" == "$(value candidate_full_tsv_sha256)" \
        && "$(canonical_value jsonl_sha256)" == "$(value candidate_full_jsonl_sha256)" \
        && "$(canonical_value summary)" == "$(value candidate_full_summary)" ]] \
        || die 'canonical Test262 baseline does not identify the frozen R3da candidate'

    verify_parent_full_source
}

verify_focused_semantics() {
    [[ "$(report_runnable "$parent_report")" == 2 \
        && "$(report_count fail-runtime "$parent_report")" == 2 \
        && "$(report_runnable "$candidate_report")" == 2 \
        && "$(report_count pass "$candidate_report")" == 2 ]] \
        || die 'R3da focused outcome counts drifted'
    awk -F'\t' '!/^#/&&!($1=="path"&&$2=="variant")&&
        !($1=="test/staging/sm/generators/delegating-yield-5.js"&&
          $2~/^(sloppy|strict)$/&&$3==""&&$4==""&&$5=="normal"&&$6==""&&
          $7=="fail-runtime"&&$8=="runtime"&&$9=="InternalError"&&
          $10=="stack overflow"){exit 2}' \
        "$parent_report" || die 'R3da parent failure frontier drifted'
    awk -F'\t' '!/^#/&&!($1=="path"&&$2=="variant")&&
        !($1=="test/staging/sm/generators/delegating-yield-5.js"&&
          $2~/^(sloppy|strict)$/&&$3==""&&$4==""&&$5=="normal"&&$6==""&&
          $7=="pass"&&$8=="normal"&&$9==""&&$10==""){exit 2}' \
        "$candidate_report" || die 'R3da candidate semantics drifted'
}

check_metadata() {
    "$runner" --suite "$suite" --validate-metadata "$tmp/metadata.bin" >/dev/null
    [[ "$(lines "$tmp/metadata.bin")" == "$(value test262_metadata_records)" \
        && "$(sha "$tmp/metadata.bin")" == "$(value test262_metadata_sha256)" ]] \
        || die 'pinned Test262 metadata drifted'
    local test_path metadata
    while IFS= read -r test_path; do
        [[ -f "$suite/$test_path" ]] || die "pinned R3da test is missing: $test_path"
        metadata=$(sed -n '/\/\*---/,/---\*\//p' "$suite/$test_path")
        ! grep -Eq '^(flags|negative|locale):' <<<"$metadata" \
            || die "R3da test gained selection-changing metadata: $test_path"
        [[ "$test_path" == test/staging/sm/generators/delegating-yield-5.js ]] \
            || die "unexpected R3da manifest path: $test_path"
    done <"$manifest"
    local fixture=$suite/test/staging/sm/generators/delegating-yield-5.js
    grep -Fq '// Test that a deep yield* chain re-yields received results without' "$fixture" \
        && grep -Fq 'return yield* n ? yield_results(expected, n - 1) : results(expected);' "$fixture" \
        && grep -Fq 'assert.deepEqual(expected, collect_results(yield_results(expected, 20)));' "$fixture" \
        || die 'R3da fixture no longer covers deep synchronous yield-star delegation'
}

verify_quickjs() {
    local test_path
    local -a files=()
    while IFS= read -r test_path; do files+=("test262/$test_path"); done <"$manifest"
    [[ -x "$source_dir/run-test262" ]] \
        || "${MAKE:-make}" -C "$source_dir" run-test262 >&2
    if ! (cd -- "$source_dir" && \
        ./run-test262 -m -c test262.conf -a -T "$workers" -f "${files[@]}") \
        >"$root/$oracle_log" 2>&1; then
        tail -n 100 "$oracle_log" >&2
        die 'pinned QuickJS could not execute the R3da manifest'
    fi
    if grep -Eq '(^|[[:space:]])FAILED($|[[:space:]])|SKIPPED FEATURE' "$oracle_log" \
        || ! grep -Fq 'Average memory statistics for 2 tests:' "$oracle_log"; then
        tail -n 100 "$oracle_log" >&2
        die 'pinned QuickJS no longer passes the R3da manifest'
    fi
}

run_report() {
    local selected_runner=$1 output=$2
    "$selected_runner" --suite "$suite" --config "$source_dir/test262.conf" \
        --oxide-profile "$profile" --manifest "$manifest" --report "$output" \
        --mode both --timeout-ms "$(value timeout_ms)" --workers "$workers" \
        --allow-failures >/dev/null
}

run_full_report() {
    "$runner" --suite "$suite" --config "$source_dir/test262.conf" \
        --oxide-profile "$profile" --all --report "$candidate_full" \
        --mode both --timeout-ms "$(value timeout_ms)" --workers "$full_workers" \
        --allow-failures >/dev/null
}

summary_json() {
    local label=$1
    awk -v summary="$(value "${label}_summary")" 'BEGIN{
        count=split(summary,items," ");printf "%s","{\"kind\":\"summary\",\"outcomes\":{";
        for(item=1;item<=count;item++){
            separator=index(items[item],"=");key=substr(items[item],1,separator-1)
            value=substr(items[item],separator+1)
            printf "%s\"%s\":%s",(item==1?"":","),key,value
        }
        print "}}"
    }'
}

reconstruct_parent_full() {
    local candidate=$1 output=$2
    local candidate_json=${candidate%.tsv}.jsonl output_json=${output%.tsv}.jsonl
    awk -F'\t' -v parent="$parent_report" \
        -v summary="# summary $(value parent_full_summary)" '
        FILENAME==parent{if(!/^#/&&!($1=="path"&&$2=="variant"))old[$1 FS $2]=$0;next}
        /^# summary /{print summary;next}
        !/^#/&&!($1=="path"&&$2=="variant"){
            key=$1 FS $2;if(key in old){print old[key];seen[key]=1;next}
        }
        {print}
        END{for(key in old)if(!(key in seen))exit 2}
    ' "$parent_report" "$candidate" >"$output" \
        || die 'could not reconstruct the R3da parent TSV'
    awk -v parent="${parent_report%.tsv}.jsonl" \
        -v summary="$(summary_json parent_full)" '
        function field(line,name,value){
            value=line;sub(".*\\\"" name "\\\":\\\"","",value);sub("\\\".*","",value);return value
        }
        FILENAME==parent{
            if($0~/^\{"kind":"result"/){key=field($0,"path") SUBSEP field($0,"variant");old[key]=$0}
            next
        }
        /^\{"kind":"summary"/{print summary;next}
        /^\{"kind":"result"/{
            key=field($0,"path") SUBSEP field($0,"variant")
            if(key in old){print old[key];seen[key]=1;next}
        }
        {print}
        END{for(key in old)if(!(key in seen))exit 2}
    ' "${parent_report%.tsv}.jsonl" "$candidate_json" >"$output_json" \
        || die 'could not reconstruct the R3da parent JSONL'
}

verify_full_join() {
    local parent=$1 candidate=$2 counts expected
    local parent_json=${parent%.tsv}.jsonl candidate_json=${candidate%.tsv}.jsonl
    rows_for_paths "$manifest" "$parent" >"$tmp/parent.cohort"
    rows_for_paths "$manifest" "$candidate" >"$tmp/candidate.cohort"
    rows_without_paths "$manifest" "$parent" >"$tmp/parent.non-cohort"
    rows_without_paths "$manifest" "$candidate" >"$tmp/candidate.non-cohort"
    report_rows "$parent_report" >"$tmp/focused.parent"
    report_rows "$candidate_report" >"$tmp/focused.candidate"
    json_rows_for_paths "$manifest" "$parent_json" >"$tmp/parent.cohort.json"
    json_rows_for_paths "$manifest" "$candidate_json" >"$tmp/candidate.cohort.json"
    json_rows_without_paths "$manifest" "$parent_json" >"$tmp/parent.non-cohort.json"
    json_rows_without_paths "$manifest" "$candidate_json" >"$tmp/candidate.non-cohort.json"
    awk '/^\{"kind":"result"/' "${parent_report%.tsv}.jsonl" >"$tmp/focused.parent.json"
    awk '/^\{"kind":"result"/' "${candidate_report%.tsv}.jsonl" >"$tmp/focused.candidate.json"
    [[ "$(lines "$tmp/parent.cohort")" == "$(value full_cohort_rows)" \
        && "$(lines "$tmp/candidate.cohort")" == "$(value full_cohort_rows)" \
        && "$(sha "$tmp/parent.cohort")" == "$(value full_parent_cohort_rows_sha256)" \
        && "$(sha "$tmp/candidate.cohort")" == "$(value full_candidate_cohort_rows_sha256)" \
        && "$(lines "$tmp/parent.non-cohort")" == "$(value full_non_cohort_rows)" \
        && "$(lines "$tmp/candidate.non-cohort")" == "$(value full_non_cohort_rows)" \
        && "$(sha "$tmp/parent.non-cohort")" == "$(value full_non_cohort_rows_sha256)" \
        && "$(sha "$tmp/candidate.non-cohort")" == "$(value full_non_cohort_rows_sha256)" \
        && "$(sha "$tmp/parent.cohort.json")" == "$(value full_parent_cohort_json_rows_sha256)" \
        && "$(sha "$tmp/candidate.cohort.json")" == "$(value full_candidate_cohort_json_rows_sha256)" \
        && "$(sha "$tmp/parent.non-cohort.json")" == "$(value full_non_cohort_json_rows_sha256)" \
        && "$(sha "$tmp/candidate.non-cohort.json")" == "$(value full_non_cohort_json_rows_sha256)" ]] \
        || die 'R3da full cohort partition drifted'
    diff -u "$tmp/focused.parent" "$tmp/parent.cohort"
    diff -u "$tmp/focused.candidate" "$tmp/candidate.cohort"
    diff -u "$tmp/parent.non-cohort" "$tmp/candidate.non-cohort"
    diff -u "$tmp/focused.parent.json" "$tmp/parent.cohort.json"
    diff -u "$tmp/focused.candidate.json" "$tmp/candidate.cohort.json"
    diff -u "$tmp/parent.non-cohort.json" "$tmp/candidate.non-cohort.json"

    counts=$(awk -F'\t' -v parent="$parent" '
        FILENAME==parent{if(!/^#/&&!($1=="path"&&$2=="variant")){old[$1 FS $2]=$0;before++}next}
        !/^#/&&!($1=="path"&&$2=="variant"){
            key=$1 FS $2;if(!(key in old))exit 2;split(old[key],a,FS)
            for(i=1;i<=6;i++)if(a[i]!=$i)exit 3
            if(a[7]=="pass"&&$7!="pass")regress++
            if(old[key]!=$0){changed++;if(a[7]!=$7)outcome++;else detail++}
            seen[key]=1
        }
        END{for(key in old)if(!(key in seen))exit 4
            printf "changed=%d outcome=%d detail=%d unchanged=%d regressions=%d",changed,outcome,detail,before-changed,regress}
    ' "$parent" "$candidate") || die 'R3da full exact join failed'
    expected="changed=$(value full_changed) outcome=$(value full_outcome_changed) detail=$(value full_detail_only) unchanged=$(value full_unchanged) regressions=$(value full_pass_regressions)"
    [[ "$counts" == "$expected" ]] || die "R3da full transition drifted: $counts"
}

cd -- "$root"
tmp=$(mktemp -d "${TMPDIR:-/tmp}/quickjs-oxide-generator-yield-star-stack-budget.XXXXXX")
trap 'rm -rf -- "$tmp"' EXIT HUP INT TERM
suite=$("$script_dir/prepare-test262.sh")
source_dir=$(dirname -- "$suite")
check_static_inputs
verify_focused_semantics
make_transition "$parent_report" "$candidate_report" "$tmp/transition.tsv"
diff -u "$transition" "$tmp/transition.tsv"

if [[ -n "$runner_override" ]]; then
    [[ -x "$runner_override" ]] || die "TEST262_RUNNER is not executable: $runner_override"
    runner=$runner_override
else
    cargo build --locked --release --quiet --bin run-test262
    runner=$root/target/release/run-test262
fi
check_metadata
verify_quickjs
if [[ "$mode" == check ]]; then
    echo 'R3da inputs verified: 1 path, 2 variants, pinned QuickJS 2/2, unchanged runner profile, checksum-bound R3cz parent receipt.'
    exit 0
fi

run_report "$runner" "$candidate_replay"
diff -u "$candidate_report" "$candidate_replay"
diff -u "${candidate_report%.tsv}.jsonl" "${candidate_replay%.tsv}.jsonl"
if [[ -n "$parent_runner_override" ]]; then
    [[ -x "$parent_runner_override" ]] \
        || die "TEST262_GENERATOR_YIELD_STAR_STACK_BUDGET_PARENT_RUNNER is not executable: $parent_runner_override"
    run_report "$parent_runner_override" "$parent_replay"
    diff -u "$parent_report" "$parent_replay"
    diff -u "${parent_report%.tsv}.jsonl" "${parent_replay%.tsv}.jsonl"
fi
make_transition "$parent_report" "$candidate_replay" "$tmp/replayed-transition.tsv"
diff -u "$transition" "$tmp/replayed-transition.tsv"
if [[ "$mode" != full ]]; then
    echo 'R3da focused gate passes: parent stack-overflow 2/2, candidate and QuickJS pass 2/2.'
    exit 0
fi

if [[ "$reuse_full_reports" == false ]]; then run_full_report; fi
verify_full_report "$candidate_full" candidate_full
parent_full=$preferred_parent_full
if [[ ! -f "$parent_full" || ! -f "${parent_full%.tsv}.jsonl" \
    || "$(sha "$parent_full")" != "$(value parent_full_tsv_sha256)" \
    || "$(sha "${parent_full%.tsv}.jsonl")" != "$(value parent_full_jsonl_sha256)" ]]; then
    parent_full=$generated_parent_full
    reconstruct_parent_full "$candidate_full" "$parent_full"
fi
verify_full_report "$parent_full" parent_full
[[ "$(report_runnable "$parent_full")" == "$(value parent_full_runnable)" \
    && "$(report_count pass "$parent_full")" == "$(value parent_full_passes)" \
    && "$(report_count fail-parse "$parent_full")" == "$(value parent_full_fail_parse)" \
    && "$(report_count fail-runtime "$parent_full")" == "$(value parent_full_fail_runtime)" \
    && "$(report_runnable "$candidate_full")" == "$(value candidate_full_runnable)" \
    && "$(report_count pass "$candidate_full")" == "$(value candidate_full_passes)" \
    && "$(report_count fail-parse "$candidate_full")" == "$(value candidate_full_fail_parse)" \
    && "$(report_count fail-runtime "$candidate_full")" == "$(value candidate_full_fail_runtime)" ]] \
    || die 'R3da full outcome counts drifted'
verify_full_join "$parent_full" "$candidate_full"
check_static_inputs
echo 'R3da full gate passes: 102037 rows, 2 generator yield-star stack-budget repairs, 102035 byte-identical non-cohort rows, zero regressions.'
