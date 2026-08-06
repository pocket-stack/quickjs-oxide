#!/usr/bin/env bash
# Reproduce the focused Test262 IsHTMLDDA host and profile admission.

set -euo pipefail
export LC_ALL=C
export TZ=America/Los_Angeles

script_dir=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd)
root=$(CDPATH='' cd -- "$script_dir/.." && pwd)
baseline=tests/test262-is-html-dda-global-baseline.txt
canonical_baseline=tests/test262-full-baseline.txt
upstream=compat/upstream.toml
live_profile=compat/test262-oxide.conf
parent_profile=tests/test262-is-html-dda-global-parent.conf
global_profile=tests/test262-is-html-dda-global-candidate.conf
scoped_profile=tests/test262-is-html-dda-scoped.conf
universe=tests/test262-is-html-dda-universe.txt
activation=tests/test262-is-html-dda-activation.txt
class_deferred=tests/test262-is-html-dda-class-deferred.txt
quickjs_receipt=tests/test262-is-html-dda-quickjs-receipt.txt
historical_report=tests/test262-is-html-dda-historical-parent.tsv
runtime_report=tests/test262-is-html-dda-runtime-parent.tsv
global_report=tests/test262-is-html-dda-global-candidate.tsv
scoped_report=tests/test262-is-html-dda-scoped.tsv
host_transition=tests/test262-is-html-dda-host-enablement-transitions.tsv
profile_transition=tests/test262-is-html-dda-profile-transitions.tsv
formal_transition=tests/test262-is-html-dda-global-transitions.tsv
scoped_transition=tests/test262-is-html-dda-scoped-transitions.tsv
workers=${TEST262_WORKERS:-8}
full_workers=${TEST262_FULL_WORKERS:-2}
reuse_full_reports=${TEST262_REUSE_FULL_REPORTS:-false}
full_report_dir=${TEST262_FULL_REPORT_DIR:-target/is-html-dda-full}
historical_full_report=${TEST262_HISTORICAL_FULL_REPORT:-$full_report_dir/historical-parent.tsv}
runtime_parent_full_report=${TEST262_RUNTIME_PARENT_FULL_REPORT:-$full_report_dir/runtime-parent.tsv}
candidate_full_report_a=${TEST262_CANDIDATE_FULL_REPORT_A:-$full_report_dir/candidate-a.tsv}
candidate_full_report_b=${TEST262_CANDIDATE_FULL_REPORT_B:-$full_report_dir/candidate-b.tsv}

baseline_lines=131
baseline_sha=2b1d6a4103a4a8ca2eaf901bddf8bcd1229ebf8db66b4228786f945dae02e2a6

usage() {
    printf 'usage: %s [--check|--full]\n' "${0##*/}"
    printf '  --check  authenticate focused inputs and frozen receipts without replay\n'
    printf '  --full   replay historical/current parents once and candidate twice across all 102037 variants\n'
    printf 'Environment for --full:\n'
    printf '  TEST262_REUSE_FULL_REPORTS=true authenticates frozen receipts without executing full vectors\n'
    printf '  TEST262_HISTORICAL_FULL_REPORT and TEST262_RUNTIME_PARENT_FULL_REPORT override parents\n'
    printf '  TEST262_CANDIDATE_FULL_REPORT_{A,B} override the candidate receipt pair\n'
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
toml_value() {
    awk -v wanted_section="[$1]" -v wanted_key="$2" '
        $0==wanted_section{inside=1;next} /^\[/{inside=0}
        inside{
            separator=index($0,"=");if(!separator)next
            key=substr($0,1,separator-1)
            gsub(/^[[:space:]]+|[[:space:]]+$/, "", key)
            if(key!=wanted_key)next
            answer=substr($0,separator+1)
            gsub(/^[[:space:]]+|[[:space:]]+$/, "", answer)
            if(answer~/^".*"$/)answer=substr(answer,2,length(answer)-2)
            print answer;found++
        }
        END{if(found!=1)exit 1}
    ' "$upstream"
}
strip_feature_lines() {
    awk -v removed="$2" '
        BEGIN{count=split(removed,item,",");for(i=1;i<=count;i++)drop[item[i]]=1}
        $0=="[features]"{inside=1;print;next}
        /^\[/{inside=0}
        inside&&($0 in drop){next}
        {print}
    ' "$1"
}
report_rows() { awk -F'\t' '!/^#/&&!($1=="path"&&$2=="variant")' "$1"; }
report_keys() { report_rows "$1" | awk -F'\t' '{print $1 "\t" $2}' | sort; }
report_identity() { report_rows "$1" | cut -f1-6; }
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
json_result_projection() {
    local report=$1
    awk -v report="$report" '
        function fail(message) {
            printf "error: IsHTMLDDA JSONL projection %s: %s\n", report, message \
                >"/dev/stderr"
            exit 2
        }
        function expect(token) {
            if(substr(line,position,length(token))!=token) {
                fail("expected " token " at column " position)
            }
            position+=length(token)
        }
        function string_value(    character,escape,digits,value) {
            expect("\"");value=""
            while(position<=length(line)) {
                character=substr(line,position,1)
                if(character=="\""){position++;return value}
                if(character=="\\") {
                    position++
                    if(position>length(line))fail("unterminated escape")
                    escape=substr(line,position,1)
                    if(escape=="u") {
                        digits=substr(line,position+1,4)
                        if(length(digits)!=4||digits~/[^0123456789abcdefABCDEF]/) {
                            fail("invalid Unicode escape")
                        }
                        value=value "\\u" digits;position+=5
                    } else {
                        if(index("\"\\/bfnrt",escape)==0)fail("invalid string escape")
                        if(escape=="\"")value=value "\""
                        else if(escape=="/")value=value "/"
                        else if(escape=="b")value=value "\\u0008"
                        else if(escape=="f")value=value "\\u000c"
                        else value=value "\\" escape
                        position++
                    }
                    continue
                }
                if(character=="\t"||character=="\r")fail("unescaped control character")
                value=value character;position++
            }
            fail("unterminated string")
        }
        function project_result(    i,key,value) {
            line=$0;position=1;expect("{")
            for(i=1;i<=11;i++) {
                if(i!=1)expect(",")
                key=string_value()
                if(key!=name[i])fail("unexpected field " key " at position " i)
                expect(":");value=string_value()
                if(i==1) {
                    if(value!="result")fail("unexpected record kind")
                } else field[i-1]=value
            }
            expect("}")
            if(position!=length(line)+1)fail("trailing record data")
            print field[1],field[2],field[3],field[4],field[5],field[6], \
                field[7],field[8],field[9],field[10]
        }
        BEGIN {
            OFS="\t"
            name[1]="kind";name[2]="path";name[3]="variant";name[4]="flags"
            name[5]="features";name[6]="expected_phase";name[7]="expected_type"
            name[8]="outcome";name[9]="actual_phase";name[10]="actual_type"
            name[11]="detail"
        }
        /^\{"kind":"metadata",/{next}
        /^\{"kind":"result",/{project_result();next}
        /^\{"kind":"summary",/{next}
        {fail("unexpected record")}
    ' "$report"
}
verify_json_projection() {
    local report=$1 label=$2 json=${1%.tsv}.jsonl
    report_rows "$report" >"$tmp/$label.tsv-projection"
    json_result_projection "$json" >"$tmp/$label.json-projection" \
        || die "JSONL result projection failed: $json"
    diff -u "$tmp/$label.tsv-projection" "$tmp/$label.json-projection" \
        || die "JSONL/TSV projection drifted: $json"
}
verify_report() {
    local report=$1 profile_sha=$2 label=$3 json=${1%.tsv}.jsonl
    verify_json_projection "$report" "$label"
    [[ "$(header "$report" quickjs)" == "$(value quickjs)" \
        && "$(header "$report" test262)" == "$(value test262)" \
        && "$(header "$report" test262_patch_sha256)" == "$(value test262_patch_sha256)" \
        && "$(header "$report" test262_config_sha256)" == "$(value test262_config_sha256)" \
        && "$(header "$report" test262_metadata_sha256)" == "$(value test262_metadata_sha256)" \
        && "$(header "$report" oxide_profile_sha256)" == "$profile_sha" \
        && "$(header "$report" profile)" == "$(value schema)" \
        && "$(header "$report" mode)" == "$(value mode)" \
        && "$(report_rows "$report" | lines /dev/stdin)" == "$(value universe_variants)" \
        && "$(report_keys "$report" | sha /dev/stdin)" == "$(value universe_keys_sha256)" \
        && "$(report_summary "$report")" == "$(computed_summary "$report")" \
        && "$(report_summary "$report")" == "$(value "${label}_summary")" \
        && "$(lines "$report")" == "$(value "${label}_report_lines")" \
        && "$(lines "$json")" == "$(value "${label}_jsonl_lines")" \
        && "$(sha "$report")" == "$(value "${label}_tsv_sha256")" \
        && "$(sha "$json")" == "$(value "${label}_jsonl_sha256")" ]] \
        || die "focused receipt drifted: $report"
}
verify_full_report() {
    local report=$1 profile_sha=$2 label=$3 json=${1%.tsv}.jsonl
    verify_json_projection "$report" "$(basename "${report%.tsv}")"
    [[ "$(header "$report" quickjs)" == "$(value quickjs)" \
        && "$(header "$report" test262)" == "$(value test262)" \
        && "$(header "$report" test262_patch_sha256)" == "$(value test262_patch_sha256)" \
        && "$(header "$report" test262_config_sha256)" == "$(value test262_config_sha256)" \
        && "$(header "$report" test262_metadata_sha256)" == "$(value test262_metadata_sha256)" \
        && "$(header "$report" oxide_profile_sha256)" == "$profile_sha" \
        && "$(header "$report" profile)" == "$(value schema)" \
        && "$(header "$report" mode)" == "$(value mode)" \
        && "$(lines "$report")" == "$(value full_report_lines)" \
        && "$(lines "$json")" == "$(value full_jsonl_lines)" \
        && "$(report_rows "$report" | lines /dev/stdin)" == "$(value full_variants)" \
        && "$(report_keys "$report" | sha /dev/stdin)" == "$(value full_keys_sha256)" \
        && "$(report_summary "$report")" == "$(computed_summary "$report")" \
        && "$(report_summary "$report")" == "$(value "${label}_summary")" \
        && "$(report_runnable "$report")" == "$(value "${label}_runnable")" \
        && "$(report_count pass "$report")" == "$(value "${label}_passes")" \
        && "$(sha "$report")" == "$(value "${label}_tsv_sha256")" \
        && "$(sha "$json")" == "$(value "${label}_jsonl_sha256")" ]] \
        || die "full receipt drifted: $report"
}
variant_keys() {
    awk -F'\t' '
        function has(list,value){return index("," list ",","," value ",")!=0}
        NR==FNR{wanted[$0]=1;next}
        $1 in wanted{
            if(has($3,"module")||has($3,"noStrict")||has($3,"raw")) \
                print $1 "\tsloppy"
            else if(has($3,"onlyStrict"))print $1 "\tstrict"
            else{print $1 "\tsloppy";print $1 "\tstrict"}
        }
    ' "$1" "$metadata_tsv" | sort
}
metadata_is_html_dda_paths() {
    awk -F'\t' '
        function has(list,value){return index("," list ",","," value ",")!=0}
        has($4,"IsHTMLDDA"){print $1}
    ' "$metadata_tsv" | sort -u
}
make_transition() {
    local before=$1 after=$2 output=$3 title=$4 before_sha=$5 after_sha=$6
    {
        echo "# $title"
        echo "# parent_commit=$(value parent_commit)"
        echo "# before_oxide_profile_sha256=$before_sha"
        echo "# after_oxide_profile_sha256=$after_sha"
        echo "# universe_sha256=$(value universe_sha256)"
        printf 'path\tvariant\tflags\tfeatures\texpected_phase\texpected_type\tbefore_outcome\tbefore_actual_phase\tbefore_actual_type\tbefore_detail\tafter_outcome\tafter_actual_phase\tafter_actual_type\tafter_detail\n'
        awk -F'\t' 'BEGIN{OFS="\t"}
            NR==FNR{
                if(!/^#/&&!($1=="path"&&$2=="variant"))old[$1 FS $2]=$0
                next
            }
            !/^#/&&!($1=="path"&&$2=="variant"){
                key=$1 FS $2;if(!(key in old))exit 2
                split(old[key],a,FS)
                for(i=1;i<=6;i++)if(a[i]!=$i)exit 3
                print $1,$2,$3,$4,$5,$6,a[7],a[8],a[9],a[10],$7,$8,$9,$10
                seen[key]=1
            }
            END{for(key in old)if(!(key in seen))exit 4}
        ' "$before" "$after"
    } >"$output"
}
transition_counts() {
    awk -F'\t' '!/^#/&&!($1=="path"&&$2=="variant"){
        different=0;for(i=7;i<=10;i++)if($i!=$(i+4))different=1
        if(different){changed++;if($7!=$11)outcome++;else detail++}else unchanged++
        if($7!="pass"&&$11=="pass")gain++
        if($7=="pass"&&$11!="pass")regression++
    } END{printf "changed=%d outcome=%d detail=%d unchanged=%d pass_gains=%d pass_regressions=%d",changed+0,outcome+0,detail+0,unchanged+0,gain+0,regression+0}' "$1"
}
full_join_counts() {
    local parent=$1 candidate=$2
    awk -F'\t' -v parent="$parent" '
        FILENAME==parent{
            if(!/^#/&&!($1=="path"&&$2=="variant")){
                old[$1 FS $2]=$0;before++
            }
            next
        }
        !/^#/&&!($1=="path"&&$2=="variant"){
            key=$1 FS $2;if(!(key in old))exit 2
            split(old[key],a,FS);for(i=1;i<=6;i++)if(a[i]!=$i)exit 3
            different=old[key]!=$0
            if(a[7]!="pass"&&$7=="pass")gain++
            if(a[7]=="pass"&&$7!="pass")regression++
            if(different){changed++;if(a[7]!=$7)outcome++;else detail++}
            seen[key]=1
        }
        END{
            for(key in old)if(!(key in seen))exit 4
            printf "changed=%d outcome=%d detail=%d unchanged=%d pass_gains=%d pass_regressions=%d",changed+0,outcome+0,detail+0,before-changed,gain+0,regression+0
        }
    ' "$parent" "$candidate"
}
verify_transition() {
    local report=$1 label=$2
    [[ "$(lines "$report")" == "$(value "${label}_transition_lines")" \
        && "$(sha "$report")" == "$(value "${label}_transition_sha256")" \
        && "$(report_rows "$report" | sha /dev/stdin)" \
            == "$(value "${label}_transition_data_sha256")" \
        && "$(transition_counts "$report")" \
            == "$(value "${label}_transition_counts")" ]] \
        || die "transition receipt drifted: $report"
}

check_static_inputs() {
    check_file "$baseline" "$baseline_lines" "$baseline_sha"
    check_file "$canonical_baseline" 8 "$(value canonical_full_baseline_sha256)"
    check_file "$parent_profile" "$(value parent_profile_lines)" \
        "$(value runtime_parent_oxide_profile_sha256)"
    check_file "$global_profile" "$(value global_candidate_profile_lines)" \
        "$(value global_candidate_oxide_profile_sha256)"
    check_file "$scoped_profile" "$(value scoped_candidate_profile_lines)" \
        "$(value scoped_candidate_oxide_profile_sha256)"
    check_file "$universe" "$(value universe_paths)" "$(value universe_sha256)"
    check_file "$activation" "$(value activation_paths)" "$(value activation_sha256)"
    check_file "$class_deferred" "$(value class_deferred_paths)" \
        "$(value class_deferred_sha256)"
    check_file "$quickjs_receipt" "$(value quickjs_receipt_lines)" \
        "$(value quickjs_receipt_sha256)"
    check_file "$historical_report" "$(value historical_parent_report_lines)" \
        "$(value historical_parent_tsv_sha256)"
    check_file "${historical_report%.tsv}.jsonl" \
        "$(value historical_parent_jsonl_lines)" \
        "$(value historical_parent_jsonl_sha256)"
    check_file "$runtime_report" "$(value runtime_parent_report_lines)" \
        "$(value runtime_parent_tsv_sha256)"
    check_file "${runtime_report%.tsv}.jsonl" "$(value runtime_parent_jsonl_lines)" \
        "$(value runtime_parent_jsonl_sha256)"
    check_file "$global_report" "$(value global_candidate_report_lines)" \
        "$(value global_candidate_tsv_sha256)"
    check_file "${global_report%.tsv}.jsonl" "$(value global_candidate_jsonl_lines)" \
        "$(value global_candidate_jsonl_sha256)"
    check_file "$scoped_report" "$(value scoped_candidate_report_lines)" \
        "$(value scoped_candidate_tsv_sha256)"
    check_file "${scoped_report%.tsv}.jsonl" "$(value scoped_candidate_jsonl_lines)" \
        "$(value scoped_candidate_jsonl_sha256)"
    check_file "$host_transition" "$(value host_transition_lines)" \
        "$(value host_transition_sha256)"
    check_file "$profile_transition" "$(value profile_transition_lines)" \
        "$(value profile_transition_sha256)"
    check_file "$formal_transition" "$(value formal_transition_lines)" \
        "$(value formal_transition_sha256)"
    check_file "$scoped_transition" "$(value scoped_transition_lines)" \
        "$(value scoped_transition_sha256)"

    cmp -s "$global_profile" "$live_profile" \
        || die 'live profile is not the admitted IsHTMLDDA global candidate'
    [[ "$(sha "$live_profile")" == "$(value live_oxide_profile_sha256)" \
        && "$(toml_value quickjs version)" == "$(value quickjs)" \
        && "$(toml_value quickjs source_sha256)" == "$(value quickjs_source_sha256)" \
        && "$(toml_value test262 commit)" == "$(value test262)" \
        && "$(toml_value test262 patch_sha256)" == "$(value test262_patch_sha256)" \
        && "$(toml_value test262 config_sha256)" == "$(value test262_config_sha256)" \
        && "$(toml_value test262 test_count)" == "$(value test262_metadata_records)" \
        && "$(toml_value test262 metadata_records_sha256)" \
            == "$(value test262_metadata_sha256)" \
        && "$(toml_value test262 oxide_profile_sha256)" \
            == "$(value live_oxide_profile_sha256)" ]] \
        || die 'pinned upstream or live profile binding drifted'
    [[ "$(canonical_value schema)" == "$(value schema)" \
        && "$(canonical_value timeout_ms)" == "$(value timeout_ms)" \
        && "$(canonical_value variants)" == "$(value full_variants)" \
        && "$(canonical_value runnable)" == "$(value global_candidate_full_runnable)" \
        && "$(canonical_value passes)" == "$(value global_candidate_full_passes)" \
        && "$(canonical_value tsv_sha256)" == "$(value global_candidate_full_tsv_sha256)" \
        && "$(canonical_value jsonl_sha256)" == "$(value global_candidate_full_jsonl_sha256)" \
        && "$(canonical_value summary)" == "$(value global_candidate_full_summary)" \
        && "$(value full_scope_variants)" == 84 \
        && "$(value full_outside_variants)" == 101953 \
        && "$(value candidate_full_replay_status)" == passed-twice \
        && "$(value candidate_full_replays)" == 2 ]] \
        || die 'IsHTMLDDA canonical full-vector bridge drifted'

    for file in "$universe" "$activation" "$class_deferred"; do
        sort -c "$file" || die "manifest is not bytewise sorted: $file"
        [[ -z "$(uniq -d "$file")" ]] || die "manifest contains duplicates: $file"
    done
    sort -u "$activation" "$class_deferred" >"$tmp/partition"
    diff -u "$universe" "$tmp/partition"
    [[ -z "$(comm -12 "$activation" "$class_deferred")" ]] \
        || die 'IsHTMLDDA activation and class-deferred partitions overlap'

    for profile in "$parent_profile" "$global_profile" "$scoped_profile"; do
        profile_section features "$profile" >"$tmp/$(basename "$profile").features"
        sort -c "$tmp/$(basename "$profile").features" \
            || die "feature section is not bytewise sorted: $profile"
    done
    [[ "$(lines "$tmp/$(basename "$parent_profile").features")" \
            == "$(value parent_features)" \
        && "$(sha "$tmp/$(basename "$parent_profile").features")" \
            == "$(value parent_features_sha256)" \
        && "$(lines "$tmp/$(basename "$global_profile").features")" \
            == "$(value global_candidate_features)" \
        && "$(sha "$tmp/$(basename "$global_profile").features")" \
            == "$(value global_candidate_features_sha256)" \
        && "$(lines "$tmp/$(basename "$scoped_profile").features")" \
            == "$(value scoped_candidate_features)" \
        && "$(sha "$tmp/$(basename "$scoped_profile").features")" \
            == "$(value scoped_candidate_features_sha256)" ]] \
        || die 'IsHTMLDDA profile feature inventory drifted'
    comm -13 "$tmp/$(basename "$parent_profile").features" \
        "$tmp/$(basename "$global_profile").features" >"$tmp/global-added"
    comm -13 "$tmp/$(basename "$parent_profile").features" \
        "$tmp/$(basename "$scoped_profile").features" >"$tmp/scoped-added"
    comm -13 "$tmp/$(basename "$global_profile").features" \
        "$tmp/$(basename "$scoped_profile").features" >"$tmp/scoped-over-global-added"
    check_file "$tmp/global-added" "$(value global_added_features)" \
        "$(value global_added_features_sha256)"
    check_file "$tmp/scoped-added" "$(value scoped_added_features)" \
        "$(value scoped_added_features_sha256)"
    check_file "$tmp/scoped-over-global-added" \
        "$(value scoped_over_global_added_features)" \
        "$(value scoped_over_global_added_features_sha256)"
    [[ "$(cat "$tmp/global-added")" == IsHTMLDDA \
        && "$(cat "$tmp/scoped-over-global-added")" == class ]] \
        || die 'IsHTMLDDA profile delta has the wrong feature names'
    strip_feature_lines "$global_profile" IsHTMLDDA >"$tmp/global-stripped"
    strip_feature_lines "$scoped_profile" IsHTMLDDA,class >"$tmp/scoped-stripped"
    diff -u "$parent_profile" "$tmp/global-stripped"
    diff -u "$parent_profile" "$tmp/scoped-stripped"
    for profile in "$parent_profile" "$global_profile" "$scoped_profile"; do
        profile_section host-agent-tests "$profile" >"$tmp/$(basename "$profile").agents"
        check_file "$tmp/$(basename "$profile").agents" "$(value host_agent_paths)" \
            "$(value host_agent_paths_sha256)"
    done

    {
        echo '# Pinned QuickJS oracle receipt for the Test262 IsHTMLDDA universe.'
        echo "quickjs=$(value quickjs)"
        echo "quickjs_source_sha256=$(value quickjs_source_sha256)"
        echo "test262=$(value test262)"
        echo "universe_sha256=$(value universe_sha256)"
        echo "source_ledger_sha256=$(value source_ledger_sha256)"
        echo "paths=$(value quickjs_paths)"
        echo "variants=$(value quickjs_variants)"
        echo "passes=$(value quickjs_passes)"
        echo 'failed=0'
        echo 'skipped_feature=0'
        echo 'result=pass'
    } >"$tmp/quickjs-receipt"
    diff -u "$quickjs_receipt" "$tmp/quickjs-receipt"
}

resolve_runner() {
    if [[ -n ${TEST262_RUNNER:-} ]]; then
        runner=$TEST262_RUNNER
        [[ -x "$runner" ]] || die "TEST262_RUNNER is not executable: $runner"
        return
    fi
    cargo build --locked --release --quiet --bin run-test262
    local target_dir=${CARGO_TARGET_DIR:-target}
    case $target_dir in /*) ;; *) target_dir=$root/$target_dir ;; esac
    runner=$target_dir/release/run-test262
}

check_metadata_and_sources() {
    "$runner" --suite "$suite" --validate-metadata "$tmp/metadata.bin" >/dev/null
    check_file "$tmp/metadata.bin" "$(value test262_metadata_records)" \
        "$(value test262_metadata_sha256)"
    tr '\0' '\t' <"$tmp/metadata.bin" >"$metadata_tsv"
    metadata_is_html_dda_paths >"$tmp/metadata-universe"
    diff -u "$universe" "$tmp/metadata-universe"
    for label in universe activation class_deferred; do
        case $label in
            universe) manifest=$universe ;;
            activation) manifest=$activation ;;
            class_deferred) manifest=$class_deferred ;;
        esac
        variant_keys "$manifest" >"$tmp/$label.keys"
        [[ "$(lines "$tmp/$label.keys")" == "$(value "${label}_variants")" \
            && "$(sha "$tmp/$label.keys")" == "$(value "${label}_keys_sha256")" ]] \
            || die "IsHTMLDDA variant-key inventory drifted: $label"
    done
    while IFS= read -r test_path; do
        [[ -f "$suite/$test_path" ]] || die "pinned source disappeared: $test_path"
        printf '%s\t%s\n' "$test_path" "$(sha "$suite/$test_path")"
    done <"$universe" >"$tmp/source-ledger"
    check_file "$tmp/source-ledger" "$(value source_ledger_lines)" \
        "$(value source_ledger_sha256)"
}

check_receipts() {
    verify_report "$historical_report" "$(value historical_parent_oxide_profile_sha256)" \
        historical_parent
    verify_report "$runtime_report" "$(value runtime_parent_oxide_profile_sha256)" \
        runtime_parent
    verify_report "$global_report" "$(value global_candidate_oxide_profile_sha256)" \
        global_candidate
    verify_report "$scoped_report" "$(value scoped_candidate_oxide_profile_sha256)" \
        scoped_candidate
    report_identity "$historical_report" >"$tmp/historical.identity"
    for report in "$runtime_report" "$global_report" "$scoped_report"; do
        report_identity "$report" >"$tmp/$(basename "$report").identity"
        diff -u "$tmp/historical.identity" "$tmp/$(basename "$report").identity"
    done
    [[ "$(report_count unsupported-host-is-html-dda "$historical_report")" == 84 \
        && "$(report_count unsupported-feature "$runtime_report")" == 84 \
        && "$(report_count pass "$global_report")" == 80 \
        && "$(report_count unsupported-feature "$global_report")" == 4 \
        && "$(report_count pass "$scoped_report")" == 84 ]] \
        || die 'four-state IsHTMLDDA focused counts drifted'
    report_rows "$historical_report" | awk -F'\t' \
        '$7!="unsupported-host-is-html-dda"||$8!="selection"||$9!="HostCapability"||$10!="missing execution capabilities: is-html-dda"{exit 1}' \
        || die 'historical IsHTMLDDA host-block receipt drifted'
    report_rows "$runtime_report" | awk -F'\t' \
        '$7!="unsupported-feature"||$8!="selection"||$9!="EngineCapability"||index($10,"IsHTMLDDA")==0{exit 1}' \
        || die 'runtime-parent IsHTMLDDA feature-block receipt drifted'
    report_rows "$global_report" | awk -F'\t' \
        '$7=="unsupported-feature"&&($8!="selection"||$9!="EngineCapability"||index($10,"class")==0){exit 1} $7!="pass"&&$7!="unsupported-feature"{exit 1}' \
        || die 'global IsHTMLDDA candidate semantics drifted'
    report_rows "$scoped_report" | awk -F'\t' '$7!="pass"{exit 1}' \
        || die 'scoped IsHTMLDDA candidate is not 84/84'
    report_rows "$global_report" | awk -F'\t' '$7=="pass"{print $1}' | sort -u \
        >"$tmp/global-pass-paths"
    report_rows "$global_report" | awk -F'\t' '$7=="unsupported-feature"{print $1}' \
        | sort -u >"$tmp/global-deferred-paths"
    diff -u "$activation" "$tmp/global-pass-paths"
    diff -u "$class_deferred" "$tmp/global-deferred-paths"
    verify_transition "$host_transition" host
    verify_transition "$profile_transition" profile
    verify_transition "$formal_transition" formal
    verify_transition "$scoped_transition" scoped
}

run_report() {
    local profile=$1 output=$2
    "$runner" --suite "$suite" --config "$source_dir/test262.conf" \
        --oxide-profile "$profile" --manifest "$universe" --report "$output" \
        --mode both --timeout-ms "$(value timeout_ms)" --workers "$workers" \
        --allow-failures >/dev/null
}
run_full_report() {
    local profile=$1 output=$2
    run_full_report_with "$runner" "$profile" "$output"
}
run_full_report_with() {
    local report_runner=$1 profile=$2 output=$3
    "$report_runner" --suite "$suite" --config "$source_dir/test262.conf" \
        --oxide-profile "$profile" --all --report "$output" \
        --mode both --timeout-ms "$(value timeout_ms)" --workers "$full_workers" \
        --allow-failures >/dev/null
}

prepare_historical_runner() {
    local parent_commit resolved_commit
    parent_commit=$(value parent_commit)
    resolved_commit=$(git rev-parse --verify "$parent_commit^{commit}") \
        || die "historical parent commit is unavailable: $parent_commit"
    [[ "$resolved_commit" == "$parent_commit" ]] \
        || die "historical parent commit did not resolve exactly: $parent_commit"
    historical_source=$tmp/historical-parent-source
    historical_target=$tmp/historical-parent-target
    historical_profile=$historical_source/compat/test262-oxide.conf
    mkdir -p "$historical_source" "$historical_target"
    git archive "$parent_commit" | tar -x -C "$historical_source"
    check_file "$historical_profile" "$(value parent_profile_lines)" \
        "$(value historical_parent_oxide_profile_sha256)"
    (
        cd -- "$historical_source"
        CARGO_TARGET_DIR="$historical_target" \
            cargo build --locked --release --quiet --bin run-test262
    )
    historical_runner=$historical_target/release/run-test262
    [[ -x "$historical_runner" ]] \
        || die 'historical parent Test262 runner was not built'
}

verify_quickjs() {
    local test_path qjs_runner=${TEST262_QUICKJS_RUNNER:-$source_dir/run-test262}
    local -a files=()
    while IFS= read -r test_path; do files+=("test262/$test_path"); done <"$universe"
    if [[ -z ${TEST262_QUICKJS_RUNNER:-} ]]; then
        [[ -x "$qjs_runner" ]] || "${MAKE:-make}" -C "$source_dir" run-test262 >&2
    fi
    [[ -x "$qjs_runner" ]] || die "QuickJS Test262 runner is not executable: $qjs_runner"
    if ! (cd -- "$source_dir" && \
        "$qjs_runner" -m -c test262.conf -a -T "$workers" -f "${files[@]}") \
        >"$tmp/quickjs.log" 2>&1; then
        tail -n 100 "$tmp/quickjs.log" >&2
        die 'pinned QuickJS could not execute the IsHTMLDDA universe'
    fi
    if grep -Eq '(^|[[:space:]])FAILED($|[[:space:]])|SKIPPED FEATURE' \
        "$tmp/quickjs.log" \
        || ! grep -Fq "Average memory statistics for $(value quickjs_passes) tests:" \
            "$tmp/quickjs.log"; then
        tail -n 100 "$tmp/quickjs.log" >&2
        die 'pinned QuickJS no longer passes the IsHTMLDDA universe 84/84'
    fi
}

replay_focused() {
    local runtime_replay=target/test262-is-html-dda-runtime-parent-replay.tsv
    local global_replay=target/test262-is-html-dda-global-candidate-replay.tsv
    local scoped_replay=target/test262-is-html-dda-scoped-replay.tsv
    run_report "$parent_profile" "$runtime_replay"
    run_report "$global_profile" "$global_replay"
    run_report "$scoped_profile" "$scoped_replay"
    for pair in \
        "$runtime_report:$runtime_replay" \
        "$global_report:$global_replay" \
        "$scoped_report:$scoped_replay"; do
        expected=${pair%%:*};actual=${pair#*:}
        cmp -s "$expected" "$actual" \
            && cmp -s "${expected%.tsv}.jsonl" "${actual%.tsv}.jsonl" \
            || die "focused replay is not byte-identical: $expected"
    done
    make_transition "$historical_report" "$runtime_replay" "$tmp/host-transition" \
        'Test262 IsHTMLDDA host-enablement transition (historical runtime to current runtime).' \
        "$(value historical_parent_oxide_profile_sha256)" \
        "$(value runtime_parent_oxide_profile_sha256)"
    make_transition "$runtime_replay" "$global_replay" "$tmp/profile-transition" \
        'Test262 IsHTMLDDA profile transition (current runtime parent to global candidate).' \
        "$(value runtime_parent_oxide_profile_sha256)" \
        "$(value global_candidate_oxide_profile_sha256)"
    make_transition "$historical_report" "$global_replay" "$tmp/formal-transition" \
        'Formal Test262 IsHTMLDDA global transition (historical parent to global candidate).' \
        "$(value historical_parent_oxide_profile_sha256)" \
        "$(value global_candidate_oxide_profile_sha256)"
    make_transition "$global_replay" "$scoped_replay" "$tmp/scoped-transition" \
        'Test262 IsHTMLDDA scoped closure transition (global candidate to exact-universe candidate).' \
        "$(value global_candidate_oxide_profile_sha256)" \
        "$(value scoped_candidate_oxide_profile_sha256)"
    diff -u "$host_transition" "$tmp/host-transition"
    diff -u "$profile_transition" "$tmp/profile-transition"
    diff -u "$formal_transition" "$tmp/formal-transition"
    diff -u "$scoped_transition" "$tmp/scoped-transition"
}

replay_full() {
    local historical_full=$historical_full_report
    local parent_full=$runtime_parent_full_report
    local candidate_full=$candidate_full_report_a
    local candidate_repeat=$candidate_full_report_b
    local candidate_json=${candidate_full%.tsv}.jsonl
    local candidate_repeat_json=${candidate_repeat%.tsv}.jsonl
    local execution_note
    mkdir -p "$(dirname -- "$historical_full")" "$(dirname -- "$parent_full")" \
        "$(dirname -- "$candidate_full")" "$(dirname -- "$candidate_repeat")"
    if [[ "$reuse_full_reports" == false ]]; then
        prepare_historical_runner
        rm -f -- \
            "$historical_full" "${historical_full%.tsv}.jsonl" \
            "$parent_full" "${parent_full%.tsv}.jsonl" \
            "$candidate_full" "${candidate_full%.tsv}.jsonl" \
            "$candidate_repeat" "${candidate_repeat%.tsv}.jsonl"
        run_full_report_with "$historical_runner" "$historical_profile" \
            "$historical_full"
        run_full_report "$parent_profile" "$parent_full"
        run_full_report "$global_profile" "$candidate_full"
        run_full_report "$global_profile" "$candidate_repeat"
        execution_note='candidate freshly executed twice'
    else
        execution_note='frozen receipts only; no full vector was executed by this invocation'
    fi
    verify_full_report "$historical_full" \
        "$(value historical_parent_oxide_profile_sha256)" historical_full
    verify_full_report "$parent_full" "$(value runtime_parent_oxide_profile_sha256)" \
        runtime_parent_full
    verify_full_report "$candidate_full" "$(value global_candidate_oxide_profile_sha256)" \
        global_candidate_full
    verify_full_report "$candidate_repeat" "$(value global_candidate_oxide_profile_sha256)" \
        global_candidate_full
    [[ ! -L "$candidate_full" && ! -L "$candidate_repeat" \
        && ! -L "$candidate_json" && ! -L "$candidate_repeat_json" ]] \
        || die 'candidate full receipts must not be symbolic links'
    if [[ "$candidate_full" -ef "$candidate_repeat" \
        || "$candidate_json" -ef "$candidate_repeat_json" ]]; then
        die 'candidate full receipts must not be the same file or hard links'
    fi
    cmp -s "$candidate_full" "$candidate_repeat" \
        && cmp -s "$candidate_json" "$candidate_repeat_json" \
        || die 'candidate full receipt pair is not byte-identical'

    for pair in \
        "$historical_full:historical-parent" \
        "$parent_full:runtime-parent" \
        "$candidate_full:global-candidate"; do
        report=${pair%%:*};label=${pair#*:}
        awk -F'\t' 'NR==FNR{wanted[$0]=1;next}
            !/^#/&&!($1=="path"&&$2=="variant")&&($1 in wanted)' \
            "$universe" "$report" >"$tmp/$label.scope"
        awk -F'\t' 'NR==FNR{wanted[$0]=1;next}
            !/^#/&&!($1=="path"&&$2=="variant")&&!($1 in wanted)' \
            "$universe" "$report" >"$tmp/$label.outside"
    done
    report_rows "$historical_report" >"$tmp/historical-focused"
    report_rows "$runtime_report" >"$tmp/runtime-focused"
    report_rows "$global_report" >"$tmp/global-focused"
    diff -u "$tmp/historical-focused" "$tmp/historical-parent.scope"
    diff -u "$tmp/runtime-focused" "$tmp/runtime-parent.scope"
    diff -u "$tmp/global-focused" "$tmp/global-candidate.scope"
    [[ "$(lines "$tmp/historical-parent.scope")" == "$(value full_scope_variants)" \
        && "$(lines "$tmp/runtime-parent.scope")" == "$(value full_scope_variants)" \
        && "$(lines "$tmp/global-candidate.scope")" == "$(value full_scope_variants)" \
        && "$(sha "$tmp/historical-parent.scope")" \
            == "$(value historical_parent_full_scope_rows_sha256)" \
        && "$(sha "$tmp/runtime-parent.scope")" \
            == "$(value runtime_parent_full_scope_rows_sha256)" \
        && "$(sha "$tmp/global-candidate.scope")" \
            == "$(value global_candidate_full_scope_rows_sha256)" \
        && "$(lines "$tmp/historical-parent.outside")" == "$(value full_outside_variants)" \
        && "$(lines "$tmp/runtime-parent.outside")" == "$(value full_outside_variants)" \
        && "$(lines "$tmp/global-candidate.outside")" == "$(value full_outside_variants)" \
        && "$(sha "$tmp/historical-parent.outside")" == "$(value full_outside_rows_sha256)" \
        && "$(sha "$tmp/runtime-parent.outside")" == "$(value full_outside_rows_sha256)" \
        && "$(sha "$tmp/global-candidate.outside")" == "$(value full_outside_rows_sha256)" ]] \
        || die 'IsHTMLDDA full-vector scope projection drifted'
    cmp -s "$tmp/historical-parent.outside" "$tmp/global-candidate.outside" \
        || die 'IsHTMLDDA historical-to-candidate join changed a row outside the exact universe'
    cmp -s "$tmp/runtime-parent.outside" "$tmp/global-candidate.outside" \
        || die 'IsHTMLDDA changed a row outside the exact 84-variant universe'
    [[ "$(full_join_counts "$historical_full" "$candidate_full")" \
        == "$(value historical_full_transition_counts)" ]] \
        || die 'IsHTMLDDA historical-to-candidate full-vector transition counts drifted'
    [[ "$(full_join_counts "$parent_full" "$candidate_full")" \
        == "$(value full_transition_counts)" ]] \
        || die 'IsHTMLDDA runtime-parent-to-candidate full-vector transition counts drifted'
    check_static_inputs
    check_receipts
    echo "IsHTMLDDA full gate passes: historical-to-candidate changed 84 outcomes; runtime-to-candidate gained 80 passes with four class reason changes; 101953 outside rows unchanged; $execution_note."
}

cd -- "$root"
tmp=$(mktemp -d "${TMPDIR:-/tmp}/quickjs-oxide-is-html-dda-global.XXXXXX")
trap 'rm -rf -- "$tmp"' EXIT HUP INT TERM
check_static_inputs
resolve_runner
suite=$("$script_dir/prepare-test262.sh")
source_dir=$(dirname -- "$suite")
metadata_tsv=$tmp/metadata.tsv
check_metadata_and_sources
check_receipts
if [[ "$mode" == check ]]; then
    echo 'IsHTMLDDA frozen receipts authenticated: historical 0/84, global 80/84, scoped 84/84, and the canonical full candidate receipt pair.'
    exit 0
fi
verify_quickjs
replay_focused
check_static_inputs
check_receipts
if [[ "$mode" == full ]]; then
    replay_full
    exit 0
fi
echo 'IsHTMLDDA focused gate passes: QuickJS 84/84, Oxide global 80/84 and scoped 84/84; canonical full receipt is checksum-bridged.'
