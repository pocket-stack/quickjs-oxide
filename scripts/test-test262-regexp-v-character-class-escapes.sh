#!/usr/bin/env bash
# Reproduce the R3ct RegExp v basic CharacterClassEscape runtime milestone.

set -euo pipefail
export LC_ALL=C
export TZ=America/Los_Angeles

script_dir=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd)
root=$(CDPATH='' cd -- "$script_dir/.." && pwd)
baseline=tests/test262-regexp-v-character-class-escapes-baseline.txt
predecessor_baseline=tests/test262-future-reserved-words-global-baseline.txt
canonical_baseline=tests/test262-full-baseline.txt
upstream=compat/upstream.toml
profile=compat/test262-oxide.conf
manifest=tests/test262-regexp-v-character-class-escapes.txt
recovery_manifest=tests/test262-regexp-v-character-class-escapes-canonical-recovery.txt
recovery_report=tests/test262-regexp-v-character-class-escapes-canonical-recovery.tsv
parent_report=tests/test262-regexp-v-character-class-escapes-parent.tsv
candidate_report=tests/test262-regexp-v-character-class-escapes-candidate.tsv
transition=tests/test262-regexp-v-character-class-escapes-transitions.tsv
candidate_output=target/test262-regexp-v-character-class-escapes-replay.tsv
parent_output=target/test262-regexp-v-character-class-escapes-parent-replay.tsv
candidate_full=target/test262-regexp-v-character-class-escapes-full.tsv
preferred_parent_full=${TEST262_REGEXP_V_PARENT_FULL:-target/test262-future-reserved-words-global-full.tsv}
generated_parent_full=target/test262-regexp-v-character-class-escapes-parent-full.tsv
oracle_log=target/test262-regexp-v-character-class-escapes-quickjs.log
workers=${TEST262_WORKERS:-8}
full_workers=${TEST262_FULL_WORKERS:-2}
reuse_full_reports=${TEST262_REUSE_FULL_REPORTS:-false}
runner_override=${TEST262_RUNNER:-}
parent_runner_override=${TEST262_REGEXP_V_PARENT_RUNNER:-}

baseline_lines=83
baseline_sha=4605dcdb55debc98b89ff60253f79e4c4f649b738c83a813f85dc46197cfb738
predecessor_lines=103
predecessor_sha=2fdb008650e965e35b3b7817a74ac19911f736e1b02085c2c2a959a2688300fc

usage() {
    printf 'usage: %s [--check|--full]\n' "${0##*/}"
    printf '  --check  verify frozen manifests, receipts, metadata, and QuickJS\n'
    printf '  --full   additionally replay and join the exact 102037-row candidate\n'
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

verify_recovery_report() {
    local json=${recovery_report%.tsv}.jsonl
    [[ -f "$recovery_report" && -f "$json" \
        && "$(header "$recovery_report" quickjs)" == "$(value quickjs)" \
        && "$(header "$recovery_report" test262)" == "$(value test262)" \
        && "$(header "$recovery_report" test262_patch_sha256)" == "$(value test262_patch_sha256)" \
        && "$(header "$recovery_report" test262_config_sha256)" == "$(value test262_config_sha256)" \
        && "$(header "$recovery_report" test262_metadata_sha256)" == "$(value test262_metadata_sha256)" \
        && "$(header "$recovery_report" oxide_profile_sha256)" == "$(value profile_sha256)" \
        && "$(header "$recovery_report" profile)" == "$(value schema)" \
        && "$(header "$recovery_report" mode)" == "$(value mode)" \
        && "$(report_rows "$recovery_report" | wc -l | tr -d '[:space:]')" == "$(value canonical_recovery_variants)" \
        && "$(report_keys "$recovery_report" | sha /dev/stdin)" == "$(value canonical_recovery_keys_sha256)" \
        && "$(report_rows "$recovery_report" | sha /dev/stdin)" == "$(value canonical_recovery_rows_sha256)" \
        && "$(report_summary "$recovery_report")" == "$(value canonical_recovery_summary)" \
        && "$(computed_summary "$recovery_report")" == "$(value canonical_recovery_summary)" \
        && "$(sha "$recovery_report")" == "$(value canonical_recovery_tsv_sha256)" \
        && "$(sha "$json")" == "$(value canonical_recovery_jsonl_sha256)" ]] \
        || die 'R3ct canonical-concurrency recovery receipt drifted'
}

verify_full_report() {
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
        && "$(report_rows "$report" | wc -l | tr -d '[:space:]')" == "$(value full_variants)" \
        && "$(report_keys "$report" | sha /dev/stdin)" == "$(value full_keys_sha256)" \
        && "$(report_summary "$report")" == "$(value "${label}_summary")" \
        && "$(computed_summary "$report")" == "$(value "${label}_summary")" \
        && "$(sha "$report")" == "$(value "${label}_tsv_sha256")" \
        && "$(sha "$json")" == "$(value "${label}_jsonl_sha256")" ]] \
        || die "full classified report drifted: $report"
}

transition_counts() {
    awk -F'\t' '!/^#/&&!($1=="path"&&$2=="variant"){
        different=0;for(i=7;i<=10;i++)if($i!=$(i+4))different=1
        if($7=="pass"&&$11!="pass")regress++
        if(different){changed++;if($7!=$11)outcome++;else detail++}else unchanged++
    } END{printf "changed=%d outcome=%d detail=%d unchanged=%d regressions=%d",changed,outcome,detail,unchanged,regress}' "$1"
}

make_transition() {
    local before=$1 after=$2 output=$3
    {
        echo '# Exhaustive R3ct RegExp v basic CharacterClassEscape runtime transition.'
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

check_manifest_and_metadata() {
    check_file "$manifest" "$(value manifest_paths)" "$(value manifest_sha256)"
    sort -c "$manifest" || die 'R3ct manifest is not bytewise sorted'
    [[ -z "$(uniq -d "$manifest")" ]] || die 'R3ct manifest contains duplicates'
    report_keys "$parent_report" >"$tmp/manifest.keys"
    [[ "$(lines "$tmp/manifest.keys")" == "$(value manifest_variants)" \
        && "$(sha "$tmp/manifest.keys")" == "$(value manifest_keys_sha256)" ]] \
        || die 'R3ct manifest variant keys drifted'

    find "$suite/test/built-ins/RegExp/CharacterClassEscapes" -maxdepth 1 \
        -type f -name '*.js' -print | sed "s#^$suite/##" | sort >"$tmp/derived.manifest"
    diff -u "$manifest" "$tmp/derived.manifest"

    check_file "$recovery_manifest" "$(value canonical_recovery_paths)" \
        "$(value canonical_recovery_manifest_sha256)"
    sort -c "$recovery_manifest" \
        || die 'R3ct canonical-recovery manifest is not bytewise sorted'
    [[ -z "$(uniq -d "$recovery_manifest")" ]] \
        || die 'R3ct canonical-recovery manifest contains duplicates'
    [[ -z "$(comm -12 "$manifest" "$recovery_manifest")" ]] \
        || die 'R3ct runtime and canonical-recovery manifests overlap'
    report_keys "$recovery_report" >"$tmp/recovery.keys"
    [[ "$(lines "$tmp/recovery.keys")" == "$(value canonical_recovery_variants)" \
        && "$(sha "$tmp/recovery.keys")" == "$(value canonical_recovery_keys_sha256)" ]] \
        || die 'R3ct canonical-recovery variant keys drifted'

    local test_path metadata
    while IFS= read -r test_path; do
        [[ -f "$suite/$test_path" ]] || die "pinned R3ct test is missing: $test_path"
        metadata=$(sed -n '/\/\*---/,/---\*\//p' "$suite/$test_path")
        [[ "$(grep -Fxc 'features: [String.fromCodePoint]' <<<"$metadata")" == 1 \
            && "$(grep -Fxc 'includes: [regExpUtils.js]' <<<"$metadata")" == 1 \
            && "$(grep -Fxc 'flags: [generated]' <<<"$metadata")" == 1 ]] \
            || die "R3ct metadata privileges drifted: $test_path"
        ! grep -Eq '^(negative|locale|es5id):' <<<"$metadata" \
            || die "R3ct test gained unexpected metadata: $test_path"
        grep -Fq 'const vflag = /' "$suite/$test_path" \
            || die "R3ct test no longer exercises a literal v regexp: $test_path"
    done <"$manifest"
    while IFS= read -r test_path; do
        [[ -f "$suite/$test_path" ]] \
            || die "pinned R3ct canonical-recovery test is missing: $test_path"
    done <"$recovery_manifest"
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
        && "$(sha "$tmp/profile.execution")" == "$(value profile_execution_sha256)" \
        && "$(grep -Fxc 'String.fromCodePoint' "$tmp/profile.features")" == 1 \
        && "$(grep -Fxc 'regexp-v-flag' "$tmp/profile.features" || true)" == 0 ]] \
        || die 'R3ct live Test262 profile drifted'
}

check_static_inputs() {
    check_file "$baseline" "$baseline_lines" "$baseline_sha"
    check_file "$predecessor_baseline" "$predecessor_lines" "$predecessor_sha"
    check_file "$parent_report" 35 "$(value parent_focused_tsv_sha256)"
    check_file "${parent_report%.tsv}.jsonl" 26 "$(value parent_focused_jsonl_sha256)"
    check_file "$candidate_report" 35 "$(value candidate_focused_tsv_sha256)"
    check_file "${candidate_report%.tsv}.jsonl" 26 "$(value candidate_focused_jsonl_sha256)"
    check_file "$transition" "$(value transition_lines)" "$(value transition_sha256)"
    verify_report "$parent_report" parent_focused
    verify_report "$candidate_report" candidate_focused
    verify_recovery_report
    check_manifest_and_metadata
    check_profile

    [[ "$(header "$transition" parent_commit)" == "$(value parent_commit)" \
        && "$(header "$transition" oxide_profile_sha256)" == "$(value profile_sha256)" \
        && "$(header "$transition" manifest_sha256)" == "$(value manifest_sha256)" \
        && "$(report_rows "$transition" | sha /dev/stdin)" == "$(value transition_data_sha256)" \
        && "$(transition_counts "$transition")" == \
            "changed=$(value transition_changed) outcome=$(value transition_outcome_changed) detail=$(value transition_detail_only) unchanged=$(value transition_unchanged) regressions=$(value pass_regressions)" \
        && "$(toml_test262_value repository)" == https://github.com/tc39/test262.git \
        && "$(toml_test262_value commit)" == "$(value test262)" \
        && "$(toml_test262_value patch_sha256)" == "$(value test262_patch_sha256)" \
        && "$(toml_test262_value config_sha256)" == "$(value test262_config_sha256)" \
        && "$(toml_test262_value test_count)" == "$(value test262_metadata_records)" \
        && "$(toml_test262_value metadata_records_sha256)" == "$(value test262_metadata_sha256)" \
        && "$(toml_test262_value oxide_profile)" == "$profile" \
        && "$(toml_test262_value oxide_profile_sha256)" == "$(value profile_sha256)" ]] \
        || die 'R3ct receipt or upstream binding drifted'

    [[ "$(predecessor_value candidate_profile_sha256)" == "$(value profile_sha256)" \
        && "$(predecessor_value candidate_full_runnable)" == "$(value parent_full_runnable)" \
        && "$(predecessor_value candidate_full_passes)" == "$(value parent_full_passes)" \
        && "$(predecessor_value candidate_full_tsv_sha256)" == "$(value parent_full_tsv_sha256)" \
        && "$(predecessor_value candidate_full_jsonl_sha256)" == "$(value parent_full_jsonl_sha256)" \
        && "$(predecessor_value candidate_full_summary)" == "$(value parent_full_summary)" \
        && "$(canonical_value schema)" == "$(value schema)" \
        && "$(canonical_value timeout_ms)" == "$(value timeout_ms)" \
        && "$(canonical_value variants)" == "$(value full_variants)" \
        && "$(canonical_value runnable)" == "$(value candidate_full_runnable)" \
        && "$(canonical_value passes)" == "$(value candidate_full_passes)" \
        && "$(canonical_value tsv_sha256)" == "$(value candidate_full_tsv_sha256)" \
        && "$(canonical_value jsonl_sha256)" == "$(value candidate_full_jsonl_sha256)" \
        && "$(canonical_value summary)" == "$(value candidate_full_summary)" \
        && "$(value candidate_full_runnable)" == "$(value parent_full_runnable)" \
        && "$(( $(value candidate_full_passes) - $(value parent_full_passes) ))" == "$(value manifest_variants)" \
        && "$(value full_changed)" == "$(value manifest_variants)" \
        && "$(value full_outcome_changed)" == "$(value manifest_variants)" \
        && "$(value full_detail_only)" == 0 \
        && "$(( $(value full_changed) + $(value full_unchanged) ))" == "$(value full_variants)" \
        && "$(value full_pass_regressions)" == 0 ]] \
        || die 'R3ct full-vector anchors drifted'
}

verify_focused_semantics() {
    [[ "$(report_runnable "$parent_report")" == "$(value parent_focused_runnable)" \
        && "$(report_count pass "$parent_report")" == "$(value parent_focused_passes)" \
        && "$(report_count unsupported-parser "$parent_report")" == "$(value parent_focused_unsupported_parser)" \
        && "$(report_runnable "$candidate_report")" == "$(value candidate_focused_runnable)" \
        && "$(report_count pass "$candidate_report")" == "$(value candidate_focused_passes)" ]] \
        || die 'R3ct focused outcome counts drifted'
    awk -F'\t' '!/^#/&&!($1=="path"&&$2=="variant")&&
        !($3=="generated"&&$4=="String.fromCodePoint"&&$5=="normal"&&$6==""&&
          $7=="unsupported-parser"&&$8=="parse"&&$9=="Unsupported"&&
          $10=="unsupported regular-expression syntax: UnicodeSetOperation at Pattern UTF-16 offset 0"){exit 2}' \
        "$parent_report" || die 'R3ct parent parser frontier drifted'
    awk -F'\t' '!/^#/&&!($1=="path"&&$2=="variant")&&
        !($3=="generated"&&$4=="String.fromCodePoint"&&$5=="normal"&&$6==""&&
          $7=="pass"&&$8=="normal"&&$9==""&&$10==""){exit 2}' \
        "$candidate_report" || die 'R3ct candidate semantics drifted'
    [[ "$(report_count pass "$recovery_report")" == "$(value canonical_recovery_variants)" ]] \
        || die 'R3ct canonical-concurrency recovery no longer passes completely'
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
        die 'pinned QuickJS could not execute the R3ct universe'
    fi
    if grep -Eq '(^|[[:space:]])FAILED($|[[:space:]])|SKIPPED FEATURE' "$oracle_log" \
        || ! grep -Fq "Average memory statistics for $(value quickjs_passes) tests:" "$oracle_log"; then
        tail -n 100 "$oracle_log" >&2
        die 'pinned QuickJS no longer passes the R3ct universe'
    fi
}

run_report() {
    local runner=$1 output=$2
    "$runner" --suite "$suite" --config "$source_dir/test262.conf" \
        --oxide-profile "$profile" --manifest "$manifest" --report "$output" \
        --mode both --timeout-ms "$(value timeout_ms)" --workers "$workers" \
        --allow-failures >/dev/null
}

run_full_report() {
    "$runner" --suite "$suite" --config "$source_dir/test262.conf" \
        --oxide-profile "$profile" --report "$candidate_full" \
        --mode both --timeout-ms "$(value timeout_ms)" --workers "$full_workers" \
        --allow-failures --all >/dev/null
}

parent_summary_json() {
    awk -v summary="$(value parent_full_summary)" 'BEGIN{
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
    [[ -f "$candidate" && -f "$candidate_json" ]] \
        || die 'cannot reconstruct the R3cs parent without a candidate full vector'
    awk -F'\t' -v parent="$parent_report" \
        -v summary="# summary $(value parent_full_summary)" '
        FILENAME==parent{
            if(!/^#/&&!($1=="path"&&$2=="variant"))old[$1 FS $2]=$0
            next
        }
        /^# summary /{print summary;next}
        !/^#/&&!($1=="path"&&$2=="variant"){
            key=$1 FS $2;if(key in old){print old[key];seen[key]=1;next}
        }
        {print}
        END{for(key in old)if(!(key in seen))exit 2}
    ' "$parent_report" "$candidate" >"$output" \
        || die 'could not reconstruct the R3cs parent TSV'

    awk -v parent="${parent_report%.tsv}.jsonl" \
        -v summary="$(parent_summary_json)" '
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
        || die 'could not reconstruct the R3cs parent JSONL'
}

verify_full_join() {
    local parent=$1 candidate=$2 counts
    rows_for_paths "$manifest" "$parent" >"$tmp/full-parent-universe.rows"
    rows_for_paths "$manifest" "$candidate" >"$tmp/full-candidate-universe.rows"
    rows_without_paths "$manifest" "$parent" >"$tmp/full-parent-non-universe.rows"
    rows_without_paths "$manifest" "$candidate" >"$tmp/full-candidate-non-universe.rows"
    report_rows "$parent_report" >"$tmp/focused-parent.rows"
    report_rows "$candidate_report" >"$tmp/focused-candidate.rows"
    [[ "$(lines "$tmp/full-parent-universe.rows")" == "$(value full_universe_rows)" \
        && "$(lines "$tmp/full-candidate-universe.rows")" == "$(value full_universe_rows)" \
        && "$(sha "$tmp/full-parent-universe.rows")" == "$(value parent_focused_rows_sha256)" \
        && "$(sha "$tmp/full-candidate-universe.rows")" == "$(value candidate_focused_rows_sha256)" \
        && "$(lines "$tmp/full-parent-non-universe.rows")" == "$(value full_non_universe_rows)" \
        && "$(lines "$tmp/full-candidate-non-universe.rows")" == "$(value full_non_universe_rows)" \
        && "$(sha "$tmp/full-parent-non-universe.rows")" == "$(value full_non_universe_rows_sha256)" \
        && "$(sha "$tmp/full-candidate-non-universe.rows")" == "$(value full_non_universe_rows_sha256)" ]] \
        || die 'R3ct full universe partition drifted'
    diff -u "$tmp/focused-parent.rows" "$tmp/full-parent-universe.rows"
    diff -u "$tmp/focused-candidate.rows" "$tmp/full-candidate-universe.rows"
    diff -u "$tmp/full-parent-non-universe.rows" "$tmp/full-candidate-non-universe.rows"

    rows_for_paths "$recovery_manifest" "$parent" >"$tmp/full-parent-recovery.rows"
    rows_for_paths "$recovery_manifest" "$candidate" >"$tmp/full-candidate-recovery.rows"
    report_rows "$recovery_report" >"$tmp/focused-recovery.rows"
    [[ "$(lines "$tmp/full-parent-recovery.rows")" == "$(value canonical_recovery_variants)" \
        && "$(lines "$tmp/full-candidate-recovery.rows")" == "$(value canonical_recovery_variants)" \
        && "$(sha "$tmp/full-parent-recovery.rows")" == "$(value canonical_recovery_rows_sha256)" \
        && "$(sha "$tmp/full-candidate-recovery.rows")" == "$(value canonical_recovery_rows_sha256)" ]] \
        || die 'R3ct canonical-concurrency recovery does not bind both full vectors'
    diff -u "$tmp/focused-recovery.rows" "$tmp/full-parent-recovery.rows"
    diff -u "$tmp/focused-recovery.rows" "$tmp/full-candidate-recovery.rows"

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
    ' "$parent" "$candidate") || die 'R3ct full exact join failed'
    local expected="changed=$(value full_changed) outcome=$(value full_outcome_changed) detail=$(value full_detail_only) unchanged=$(value full_unchanged) regressions=$(value full_pass_regressions)"
    [[ "$counts" == "$expected" ]] || die "R3ct full transition drifted: $counts"
}

cd -- "$root"
tmp=$(mktemp -d "${TMPDIR:-/tmp}/quickjs-oxide-regexp-v-escapes.XXXXXX")
trap 'rm -rf -- "$tmp"' EXIT HUP INT TERM
suite=$("$script_dir/prepare-test262.sh")
source_dir=$(dirname -- "$suite")
check_static_inputs
verify_focused_semantics
make_transition "$parent_report" "$candidate_report" "$tmp/transition.tsv"
diff -u "$transition" "$tmp/transition.tsv"
verify_quickjs
if [[ "$mode" == check ]]; then
    echo 'R3ct inputs verified: 12 paths, 24 variants, QuickJS 24/24, checksum-bound parent/candidate receipts.'
    exit 0
fi

if [[ -n "$runner_override" ]]; then
    [[ -x "$runner_override" ]] || die "TEST262_RUNNER is not executable: $runner_override"
    runner=$runner_override
else
    cargo build --locked --release --quiet --bin run-test262
    runner=$root/target/release/run-test262
fi
"$runner" --suite "$suite" --validate-metadata "$tmp/metadata.bin" >/dev/null
[[ "$(lines "$tmp/metadata.bin")" == "$(value test262_metadata_records)" \
    && "$(sha "$tmp/metadata.bin")" == "$(value test262_metadata_sha256)" ]] \
    || die 'pinned Test262 metadata drifted'
run_report "$runner" "$candidate_output"
diff -u "$candidate_report" "$candidate_output"
diff -u "${candidate_report%.tsv}.jsonl" "${candidate_output%.tsv}.jsonl"
if [[ -n "$parent_runner_override" ]]; then
    [[ -x "$parent_runner_override" ]] \
        || die "TEST262_REGEXP_V_PARENT_RUNNER is not executable: $parent_runner_override"
    run_report "$parent_runner_override" "$parent_output"
    diff -u "$parent_report" "$parent_output"
    diff -u "${parent_report%.tsv}.jsonl" "${parent_output%.tsv}.jsonl"
fi
make_transition "$parent_report" "$candidate_output" "$tmp/replayed-transition.tsv"
diff -u "$transition" "$tmp/replayed-transition.tsv"
check_static_inputs
verify_focused_semantics
if [[ "$mode" != full ]]; then
    echo 'R3ct focused semantics pass: QuickJS 24/24, Oxide 24/24, 24 runtime repairs, zero regressions.'
    exit 0
fi

if [[ "$reuse_full_reports" == false ]]; then run_full_report; fi
verify_full_report "$candidate_full" candidate_full
[[ "$(report_runnable "$candidate_full")" == "$(value candidate_full_runnable)" \
    && "$(report_count pass "$candidate_full")" == "$(value candidate_full_passes)" ]] \
    || die 'R3ct candidate full outcome counts drifted'
parent_full=$preferred_parent_full
if [[ ! -f "$parent_full" || ! -f "${parent_full%.tsv}.jsonl" \
    || "$(sha "$parent_full")" != "$(value parent_full_tsv_sha256)" \
    || "$(sha "${parent_full%.tsv}.jsonl")" != "$(value parent_full_jsonl_sha256)" ]]; then
    parent_full=$generated_parent_full
    reconstruct_parent_full "$candidate_full" "$parent_full"
fi
verify_full_report "$parent_full" parent_full
[[ "$(report_runnable "$parent_full")" == "$(value parent_full_runnable)" \
    && "$(report_count pass "$parent_full")" == "$(value parent_full_passes)" ]] \
    || die 'R3cs parent full outcome counts drifted'
verify_full_join "$parent_full" "$candidate_full"
echo 'R3ct full semantics pass: 102037 rows, 24 runtime repairs, 102013 unchanged, zero pass regressions.'
