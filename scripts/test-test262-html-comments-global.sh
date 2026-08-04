#!/usr/bin/env bash
# Reproduce the R3co global admission of audited HTML-like-comment negatives.

set -euo pipefail
export LC_ALL=C
export TZ=America/Los_Angeles

script_dir=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd)
root=$(CDPATH='' cd -- "$script_dir/.." && pwd)
baseline=tests/test262-html-comments-global-baseline.txt
canonical_baseline=tests/test262-full-baseline.txt
upstream=compat/upstream.toml
live_profile=compat/test262-oxide.conf
parent_profile=tests/test262-html-comments-global-parent.conf
candidate_profile=tests/test262-html-comments-global-candidate.conf
added_negatives=tests/test262-html-comments-global-added-negatives.txt
universe=tests/test262-html-comments.txt
runtime_activation=tests/test262-html-comments-runtime-activation.txt
already_pass=tests/test262-html-comments-already-pass.txt
module_unchanged=tests/test262-html-comments-module-unchanged.txt
parent_report=tests/test262-html-comments-global-parent.tsv
transition=tests/test262-html-comments-global-transitions.tsv
candidate_report=target/test262-html-comments-global-candidate.tsv
candidate_full=target/test262-html-comments-global-full.tsv
preferred_parent_full=${TEST262_HTML_COMMENTS_PARENT_FULL:-target/test262-html-comments-runtime-full.tsv}
generated_parent_full=target/test262-html-comments-global-parent-full.tsv
oracle_log=target/test262-html-comments-global-quickjs.log
workers=${TEST262_WORKERS:-8}
full_workers=${TEST262_FULL_WORKERS:-2}
reuse_full_reports=${TEST262_REUSE_FULL_REPORTS:-false}
runner_override=${TEST262_RUNNER:-}

baseline_lines=101
baseline_sha=f71218ae52bdf5ac8579ee7ee0b416be00435ed61fb26c62b9d46a50560ad821

usage() {
    printf 'usage: %s [--check|--full]\n' "${0##*/}"
    printf '  --check  verify frozen profiles, manifests, receipts, and QuickJS oracle\n'
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
canonical_value() { value_from "$canonical_baseline" "$1"; }
has_value() { awk -F= -v wanted="$2" '$1==wanted{found++} END{exit found==1?0:1}' "$1"; }
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
manifest_keys() {
    awk -F'\t' 'NR==FNR{if(NF&&$1!~/^#/)wanted[$1]=1;next}
        !/^#/&&!($1=="path"&&$2=="variant")&&($1 in wanted){print $1 "\t" $2}' \
        "$1" "$parent_report" | sort
}
toml_test262_value() {
    awk -v wanted="$2" '
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
    ' "$1"
}

verify_report() {
    local report=$1 profile_sha=$2 rows=$3 keys_sha=$4 summary=$5 tsv_sha=$6 json_sha=$7
    local json=${report%.tsv}.jsonl
    [[ -f "$report" && -f "$json" \
        && "$(header "$report" quickjs)" == "$(value quickjs)" \
        && "$(header "$report" test262)" == "$(value test262)" \
        && "$(header "$report" test262_patch_sha256)" == "$(value test262_patch_sha256)" \
        && "$(header "$report" test262_config_sha256)" == "$(value test262_config_sha256)" \
        && "$(header "$report" test262_metadata_sha256)" == "$(value test262_metadata_sha256)" \
        && "$(header "$report" oxide_profile_sha256)" == "$profile_sha" \
        && "$(header "$report" profile)" == "$(value schema)" \
        && "$(header "$report" mode)" == "$(value mode)" \
        && "$(report_rows "$report" | wc -l | tr -d '[:space:]')" == "$rows" \
        && "$(report_keys "$report" | sha /dev/stdin)" == "$keys_sha" \
        && "$(report_summary "$report")" == "$summary" \
        && "$(computed_summary "$report")" == "$summary" \
        && "$(sha "$report")" == "$tsv_sha" \
        && "$(sha "$json")" == "$json_sha" ]] \
        || die "classified report drifted: $report"
}

check_profiles() {
    check_file "$parent_profile" "$(value parent_profile_lines)" \
        "$(value parent_profile_sha256)"
    check_file "$candidate_profile" "$(value candidate_profile_lines)" \
        "$(value candidate_profile_sha256)"
    check_file "$live_profile" "$(value candidate_profile_lines)" \
        "$(value candidate_profile_sha256)"
    cmp -s "$candidate_profile" "$live_profile" \
        || die 'live profile is not byte-identical to the R3co candidate'

    for section in features audited-negative-tests execution; do
        profile_section "$section" "$parent_profile" >"$tmp/parent.$section"
        profile_section "$section" "$candidate_profile" >"$tmp/candidate.$section"
        sort -c "$tmp/parent.$section" \
            || die "parent $section profile section is not bytewise sorted"
        sort -c "$tmp/candidate.$section" \
            || die "candidate $section profile section is not bytewise sorted"
    done
    [[ "$(lines "$tmp/parent.features")" == "$(value profile_features)" \
        && "$(lines "$tmp/candidate.features")" == "$(value profile_features)" \
        && "$(sha "$tmp/parent.features")" == "$(value profile_features_sha256)" \
        && "$(sha "$tmp/candidate.features")" == "$(value profile_features_sha256)" \
        && "$(lines "$tmp/parent.audited-negative-tests")" \
            == "$(value parent_audited_negative_tests)" \
        && "$(sha "$tmp/parent.audited-negative-tests")" \
            == "$(value parent_audited_negative_tests_sha256)" \
        && "$(lines "$tmp/candidate.audited-negative-tests")" \
            == "$(value candidate_audited_negative_tests)" \
        && "$(sha "$tmp/candidate.audited-negative-tests")" \
            == "$(value candidate_audited_negative_tests_sha256)" \
        && "$(lines "$tmp/parent.execution")" == "$(value profile_execution_entries)" \
        && "$(lines "$tmp/candidate.execution")" == "$(value profile_execution_entries)" \
        && "$(sha "$tmp/parent.execution")" == "$(value profile_execution_sha256)" \
        && "$(sha "$tmp/candidate.execution")" == "$(value profile_execution_sha256)" ]] \
        || die 'R3co profile inventory drifted'
    cmp -s "$tmp/parent.features" "$tmp/candidate.features" \
        || die 'R3co changes the feature profile section'
    cmp -s "$tmp/parent.execution" "$tmp/candidate.execution" \
        || die 'R3co changes the execution profile section'
    comm -23 "$tmp/parent.audited-negative-tests" \
        "$tmp/candidate.audited-negative-tests" >"$tmp/removed.negatives"
    comm -13 "$tmp/parent.audited-negative-tests" \
        "$tmp/candidate.audited-negative-tests" >"$tmp/added.negatives"
    [[ ! -s "$tmp/removed.negatives" ]] \
        || die 'R3co removes an audited negative Test262 path'
    diff -u "$added_negatives" "$tmp/added.negatives"
}

check_manifests() {
    for spec in \
        universe:$universe \
        added_negatives:$added_negatives \
        runtime_activation:$runtime_activation \
        already_pass:$already_pass \
        module_unchanged:$module_unchanged; do
        prefix=${spec%%:*}
        file=${spec#*:}
        check_file "$file" "$(value "${prefix}_paths")" \
            "$(value "${prefix}_paths_sha256")"
        sort -c "$file" || die "manifest is not bytewise sorted: $file"
        [[ -z "$(uniq -d "$file")" ]] || die "manifest contains duplicates: $file"
        manifest_keys "$file" >"$tmp/$prefix.keys"
        [[ "$(lines "$tmp/$prefix.keys")" == "$(value "${prefix}_variants")" \
            && "$(sha "$tmp/$prefix.keys")" == "$(value "${prefix}_keys_sha256")" ]] \
            || die "manifest variant keys drifted: $file"
    done
    cat "$runtime_activation" "$already_pass" "$added_negatives" "$module_unchanged" \
        | sort >"$tmp/universe.partition"
    diff -u "$universe" "$tmp/universe.partition"
    [[ -z "$(uniq -d "$tmp/universe.partition")" ]] \
        || die 'R3co HTML comment manifest partitions overlap'
}

transition_counts() {
    awk -F'\t' '!/^#/&&!($1=="path"&&$2=="variant"){
        different=0;for(i=7;i<=10;i++)if($i!=$(i+4))different=1
        if(different){changed++;if($7!=$11)outcome++;else detail++}else unchanged++
    } END{printf "changed=%d outcome=%d detail=%d unchanged=%d",changed,outcome,detail,unchanged}' "$1"
}

make_transition() {
    local before=$1 after=$2 output=$3
    {
        echo '# Exhaustive R3co HTML-like-comments negative-test global-admission transition.'
        echo "# parent_profile_sha256=$(value parent_profile_sha256)"
        echo "# candidate_profile_sha256=$(value candidate_profile_sha256)"
        echo "# manifest_sha256=$(value universe_paths_sha256)"
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

canonical_matches_parent() {
    [[ "$(canonical_value schema)" == "$(value schema)" \
        && "$(canonical_value timeout_ms)" == "$(value timeout_ms)" \
        && "$(canonical_value variants)" == "$(value full_variants)" \
        && "$(canonical_value runnable)" == "$(value parent_full_runnable)" \
        && "$(canonical_value passes)" == "$(value parent_full_passes)" \
        && "$(canonical_value tsv_sha256)" == "$(value parent_full_tsv_sha256)" \
        && "$(canonical_value jsonl_sha256)" == "$(value parent_full_jsonl_sha256)" \
        && "$(canonical_value summary)" == "$(value parent_full_summary)" ]]
}

canonical_matches_candidate() {
    has_value "$baseline" candidate_full_tsv_sha256 || return 1
    [[ "$(canonical_value schema)" == "$(value schema)" \
        && "$(canonical_value timeout_ms)" == "$(value timeout_ms)" \
        && "$(canonical_value variants)" == "$(value full_variants)" \
        && "$(canonical_value runnable)" == "$(value candidate_full_runnable)" \
        && "$(canonical_value passes)" == "$(value candidate_full_passes)" \
        && "$(canonical_value tsv_sha256)" == "$(value candidate_full_tsv_sha256)" \
        && "$(canonical_value jsonl_sha256)" == "$(value candidate_full_jsonl_sha256)" \
        && "$(canonical_value summary)" == "$(value candidate_full_summary)" ]]
}

check_canonical_binding() {
    if has_value "$baseline" candidate_full_tsv_sha256; then
        canonical_matches_candidate \
            || die 'canonical Test262 baseline does not identify the frozen R3co candidate'
    else
        canonical_matches_parent \
            || die 'pre-full canonical Test262 baseline does not identify the R3cn parent'
    fi
}

check_static_inputs() {
    check_file "$baseline" "$baseline_lines" "$baseline_sha"
    check_file "$parent_report" 43 "$(value parent_focused_tsv_sha256)"
    check_file "${parent_report%.tsv}.jsonl" 34 \
        "$(value parent_focused_jsonl_sha256)"
    check_file "$transition" "$(value transition_lines)" \
        "$(value transition_receipt_sha256)"
    check_profiles
    check_manifests
    verify_report "$parent_report" "$(value parent_profile_sha256)" \
        "$(value universe_variants)" "$(value universe_keys_sha256)" \
        "$(value parent_focused_summary)" "$(value parent_focused_tsv_sha256)" \
        "$(value parent_focused_jsonl_sha256)"
    [[ "$(report_rows "$transition" | sha /dev/stdin)" \
            == "$(value transition_data_sha256)" \
        && "$(transition_counts "$transition")" == \
            "changed=$(value transition_changed) outcome=$(value transition_outcome_changed) detail=$(value transition_detail_only) unchanged=$(value transition_unchanged)" \
        && "$(toml_test262_value "$upstream" repository)" \
            == https://github.com/tc39/test262.git \
        && "$(toml_test262_value "$upstream" commit)" == "$(value test262)" \
        && "$(toml_test262_value "$upstream" patch_sha256)" \
            == "$(value test262_patch_sha256)" \
        && "$(toml_test262_value "$upstream" config_sha256)" \
            == "$(value test262_config_sha256)" \
        && "$(toml_test262_value "$upstream" test_count)" \
            == "$(value test262_metadata_records)" \
        && "$(toml_test262_value "$upstream" metadata_records_sha256)" \
            == "$(value test262_metadata_sha256)" \
        && "$(toml_test262_value "$upstream" oxide_profile)" == "$live_profile" \
        && "$(toml_test262_value "$upstream" oxide_profile_sha256)" \
            == "$(value candidate_profile_sha256)" ]] \
        || die 'R3co upstream or transition binding drifted'
    check_canonical_binding
}

verify_quickjs() {
    local test_path
    local -a files=()
    while IFS= read -r test_path; do files+=("test262/$test_path"); done <"$universe"
    [[ -x "$source_dir/run-test262" ]] \
        || "${MAKE:-make}" -C "$source_dir" run-test262 >&2
    if ! (cd -- "$source_dir" && \
        ./run-test262 -m -c test262.conf -a -T "$workers" -f "${files[@]}") \
        >"$root/$oracle_log" 2>&1; then
        tail -n 100 "$oracle_log" >&2
        die 'pinned QuickJS could not execute the HTML comment universe'
    fi
    if grep -Eq '(^|[[:space:]])FAILED($|[[:space:]])|SKIPPED FEATURE' "$oracle_log" \
        || ! grep -Fq "Average memory statistics for $(value quickjs_passes) tests:" \
            "$oracle_log"; then
        tail -n 100 "$oracle_log" >&2
        die 'pinned QuickJS no longer passes the HTML comment universe'
    fi
}

run_report() {
    local profile=$1 output=$2 scope=$3 pool=$4
    local -a args=(--suite "$suite" --config "$source_dir/test262.conf"
        --oxide-profile "$profile" --report "$output" --mode both
        --timeout-ms "$(value timeout_ms)" --workers "$pool" --allow-failures)
    if [[ "$scope" == full ]]; then args+=(--all); else args+=(--manifest "$universe"); fi
    "$runner" "${args[@]}" >/dev/null
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
        || die 'cannot reconstruct the R3cn parent without a candidate full vector'
    awk -F'\t' -v parent="$parent_report" \
        -v profile="# oxide_profile_sha256=$(value parent_profile_sha256)" \
        -v summary="# summary $(value parent_full_summary)" '
        FILENAME==parent{
            if(!/^#/&&!($1=="path"&&$2=="variant"))old[$1 FS $2]=$0
            next
        }
        /^# oxide_profile_sha256=/{print profile;next}
        /^# summary /{print summary;next}
        !/^#/&&!($1=="path"&&$2=="variant"){
            key=$1 FS $2;if(key in old){print old[key];seen[key]=1;next}
        }
        {print}
        END{for(key in old)if(!(key in seen))exit 2}
    ' "$parent_report" "$candidate" >"$output" \
        || die 'could not reconstruct the R3cn parent TSV'

    awk -v parent="${parent_report%.tsv}.jsonl" \
        -v profile="$(value parent_profile_sha256)" \
        -v summary="$(parent_summary_json)" '
        function field(line,name,value){
            value=line;sub(".*\\\"" name "\\\":\\\"","",value);sub("\\\".*","",value);return value
        }
        FILENAME==parent{
            if($0~/^\{"kind":"result"/){key=field($0,"path") SUBSEP field($0,"variant");old[key]=$0}
            next
        }
        /^\{"kind":"metadata"/{
            sub(/"oxide_profile_sha256":"[^"]*"/,"\"oxide_profile_sha256\":\"" profile "\"");print;next
        }
        /^\{"kind":"summary"/{print summary;next}
        /^\{"kind":"result"/{
            key=field($0,"path") SUBSEP field($0,"variant")
            if(key in old){print old[key];seen[key]=1;next}
        }
        {print}
        END{for(key in old)if(!(key in seen))exit 2}
    ' "${parent_report%.tsv}.jsonl" "$candidate_json" >"$output_json" \
        || die 'could not reconstruct the R3cn parent JSONL'
}

verify_full_join() {
    local parent=$1 candidate=$2 counts
    rows_for_paths "$universe" "$parent" >"$tmp/full-parent-universe.rows"
    rows_for_paths "$universe" "$candidate" >"$tmp/full-candidate-universe.rows"
    rows_without_paths "$universe" "$parent" >"$tmp/full-parent-non-universe.rows"
    rows_without_paths "$universe" "$candidate" >"$tmp/full-candidate-non-universe.rows"
    report_rows "$parent_report" >"$tmp/focused-parent.rows"
    report_rows "$candidate_report" >"$tmp/focused-candidate.rows"
    [[ "$(lines "$tmp/full-parent-universe.rows")" == "$(value full_universe_rows)" \
        && "$(lines "$tmp/full-candidate-universe.rows")" == "$(value full_universe_rows)" \
        && "$(sha "$tmp/full-parent-universe.rows")" \
            == "$(value full_parent_universe_rows_sha256)" \
        && "$(sha "$tmp/full-candidate-universe.rows")" \
            == "$(value full_candidate_universe_rows_sha256)" \
        && "$(lines "$tmp/full-parent-non-universe.rows")" \
            == "$(value full_non_universe_rows)" \
        && "$(lines "$tmp/full-candidate-non-universe.rows")" \
            == "$(value full_non_universe_rows)" \
        && "$(sha "$tmp/full-parent-non-universe.rows")" \
            == "$(value full_non_universe_rows_sha256)" \
        && "$(sha "$tmp/full-candidate-non-universe.rows")" \
            == "$(value full_non_universe_rows_sha256)" ]] \
        || die 'R3co full universe partition drifted'
    diff -u "$tmp/focused-parent.rows" "$tmp/full-parent-universe.rows"
    diff -u "$tmp/focused-candidate.rows" "$tmp/full-candidate-universe.rows"
    diff -u "$tmp/full-parent-non-universe.rows" \
        "$tmp/full-candidate-non-universe.rows"

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
    ' "$parent" "$candidate") || die 'R3co full exact join failed'
    local expected="changed=$(value full_changed) outcome=$(value full_outcome_changed) detail=$(value full_detail_only) unchanged=$(value full_unchanged) regressions=$(value full_pass_regressions)"
    [[ "$counts" == "$expected" ]] || die "R3co full transition drifted: $counts"
}

verify_focused_semantics() {
    [[ "$(report_runnable "$candidate_report")" == "$(value candidate_focused_runnable)" \
        && "$(report_count pass "$candidate_report")" == "$(value candidate_focused_passes)" \
        && "$(report_count unsupported-module "$candidate_report")" \
            == "$(value candidate_focused_unsupported_module)" ]] \
        || die 'R3co focused outcome counts drifted'
    for manifest in "$runtime_activation" "$already_pass"; do
        awk -F'\t' 'NR==FNR{wanted[$0]=1;next}
            !/^#/&&!($1=="path"&&$2=="variant")&&($1 in wanted)&&
            !($7=="pass"&&$8=="normal"&&$9==""){exit 2}' \
            "$manifest" "$candidate_report" \
            || die "R3co existing HTML comment pass semantics drifted: $manifest"
    done
    awk -F'\t' 'NR==FNR{wanted[$0]=1;next}
        !/^#/&&!($1=="path"&&$2=="variant")&&($1 in wanted)&&
        !($7=="unsupported-negative-provenance"&&$8=="selection"&&$9=="EngineCapability"){exit 2}' \
        "$added_negatives" "$parent_report" \
        || die 'R3co parent negative provenance frontier drifted'
    awk -F'\t' 'NR==FNR{wanted[$0]=1;next}
        !/^#/&&!($1=="path"&&$2=="variant")&&($1 in wanted)&&
        !($7=="pass"&&$8==$5&&$9==$6){exit 2}' \
        "$added_negatives" "$candidate_report" \
        || die 'R3co admitted negative phase/type semantics drifted'
    awk -F'\t' 'NR==FNR{wanted[$0]=1;next}
        !/^#/&&!($1=="path"&&$2=="variant")&&($1 in wanted)&&$7!="unsupported-module"{exit 2}' \
        "$module_unchanged" "$candidate_report" \
        || die 'R3co module frontier drifted'
}

cd -- "$root"
tmp=$(mktemp -d "${TMPDIR:-/tmp}/quickjs-oxide-html-comments-global.XXXXXX")
trap 'rm -rf -- "$tmp"' EXIT HUP INT TERM
check_static_inputs
suite=$("$script_dir/prepare-test262.sh")
source_dir=$(dirname -- "$suite")
git -C "$suite" ls-files \
    'test/annexB/built-ins/Function/*html*comment*.js' \
    'test/annexB/language/comments/*html*.js' \
    'test/language/comments/*html*.js' \
    'test/language/module-code/comment*html*.js' | sort >"$tmp/derived-universe"
diff -u "$universe" "$tmp/derived-universe"
verify_quickjs
if [[ "$mode" == check ]]; then
    echo 'R3co HTML comment inputs verified: 19 paths, 32 variants, QuickJS 32/32, checksum-bound R3cn parent and R3co candidate.'
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
run_report "$parent_profile" "$tmp/parent.tsv" focused "$workers"
run_report "$candidate_profile" "$candidate_report" focused "$workers"
verify_report "$tmp/parent.tsv" "$(value parent_profile_sha256)" \
    "$(value universe_variants)" "$(value universe_keys_sha256)" \
    "$(value parent_focused_summary)" "$(value parent_focused_tsv_sha256)" \
    "$(value parent_focused_jsonl_sha256)"
diff -u "$parent_report" "$tmp/parent.tsv"
diff -u "${parent_report%.tsv}.jsonl" "${tmp}/parent.jsonl"
verify_report "$candidate_report" "$(value candidate_profile_sha256)" \
    "$(value universe_variants)" "$(value universe_keys_sha256)" \
    "$(value candidate_focused_summary)" "$(value candidate_focused_tsv_sha256)" \
    "$(value candidate_focused_jsonl_sha256)"
verify_focused_semantics
make_transition "$tmp/parent.tsv" "$candidate_report" "$tmp/transition.tsv"
diff -u "$transition" "$tmp/transition.tsv"
[[ "$(transition_counts "$tmp/transition.tsv")" == \
    "changed=$(value transition_changed) outcome=$(value transition_outcome_changed) detail=$(value transition_detail_only) unchanged=$(value transition_unchanged)" ]] \
    || die 'R3co focused transition semantics drifted'
check_static_inputs
if [[ "$mode" != full ]]; then
    echo 'R3co HTML comment focused semantics pass: QuickJS 32/32, Oxide 17 newly admitted negative variants, 15 rows unchanged.'
    exit 0
fi

for required in \
    candidate_full_runnable candidate_full_passes candidate_full_tsv_sha256 \
    candidate_full_jsonl_sha256 candidate_full_summary full_universe_rows \
    full_parent_universe_rows_sha256 full_candidate_universe_rows_sha256 \
    full_non_universe_rows full_non_universe_rows_sha256 full_changed \
    full_outcome_changed full_detail_only full_unchanged full_pass_regressions; do
    has_value "$baseline" "$required" \
        || die "R3co baseline has not frozen the full receipt field: $required"
done
if [[ "$reuse_full_reports" == false ]]; then
    run_report "$candidate_profile" "$candidate_full" full "$full_workers"
fi
verify_report "$candidate_full" "$(value candidate_profile_sha256)" \
    "$(value full_variants)" "$(value full_keys_sha256)" \
    "$(value candidate_full_summary)" "$(value candidate_full_tsv_sha256)" \
    "$(value candidate_full_jsonl_sha256)"
[[ "$(report_runnable "$candidate_full")" == "$(value candidate_full_runnable)" \
    && "$(report_count pass "$candidate_full")" == "$(value candidate_full_passes)" ]] \
    || die 'R3co candidate full outcome counts drifted'
canonical_matches_candidate \
    || die 'canonical Test262 baseline does not identify the R3co candidate'
parent_full=$preferred_parent_full
if [[ ! -f "$parent_full" || ! -f "${parent_full%.tsv}.jsonl" \
    || "$(sha "$parent_full")" != "$(value parent_full_tsv_sha256)" \
    || "$(sha "${parent_full%.tsv}.jsonl")" != "$(value parent_full_jsonl_sha256)" ]]; then
    parent_full=$generated_parent_full
    reconstruct_parent_full "$candidate_full" "$parent_full"
fi
verify_report "$parent_full" "$(value parent_profile_sha256)" \
    "$(value full_variants)" "$(value full_keys_sha256)" \
    "$(value parent_full_summary)" "$(value parent_full_tsv_sha256)" \
    "$(value parent_full_jsonl_sha256)"
[[ "$(report_runnable "$parent_full")" == "$(value parent_full_runnable)" \
    && "$(report_count pass "$parent_full")" == "$(value parent_full_passes)" ]] \
    || die 'R3cn parent full outcome counts drifted'
verify_full_join "$parent_full" "$candidate_full"
echo 'R3co HTML comment full semantics pass: 102037 rows, 17 newly admitted negative variants, 102020 unchanged, zero pass regressions.'
