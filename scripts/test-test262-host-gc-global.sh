#!/usr/bin/env bash
# Reproduce the R3ch global admission of the Test262 $262.gc host hook.

set -euo pipefail
export LC_ALL=C
export TZ=America/Los_Angeles

script_dir=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd)
root=$(CDPATH='' cd -- "$script_dir/.." && pwd)
baseline=tests/test262-host-gc-global-baseline.txt
canonical_baseline=tests/test262-full-baseline.txt
parent=tests/test262-host-gc-global-parent.conf
candidate=tests/test262-host-gc-global-candidate.conf
live_profile=compat/test262-oxide.conf
upstream=compat/upstream.toml
universe=tests/test262-host-gc-universe.txt
activation=tests/test262-host-gc-activation.txt
create_realm=tests/test262-host-gc-create-realm-deferred.txt
historical_parent=tests/test262-host-gc-parent.tsv
historical_parent_json=tests/test262-host-gc-parent.jsonl
transition=tests/test262-host-gc-global-transitions.tsv
parent_report=target/test262-host-gc-global-parent.tsv
candidate_report=target/test262-host-gc-global-candidate.tsv
parent_full=target/test262-host-gc-global-parent-full.tsv
candidate_full=target/test262-host-gc-global-candidate-full.tsv
workers=${TEST262_WORKERS:-8}
full_workers=${TEST262_FULL_WORKERS:-2}
reuse_full_reports=${TEST262_REUSE_FULL_REPORTS:-false}

quickjs=2026-06-04
test262=5c8206929d81b2d3d727ca6aac56c18358c8d790
patch_sha=f4b23b04641d438df0826fb17d7a5db276af2bdb085b42cc09aa8d50e0da9ba3
config_sha=79c64748ff1182baf5433d0a8378e3666738a785d02faf71f0d459ed42ae897b
metadata_sha=a37219960819e56a5c5c1723d31d6a33095c778bf5347385187fde96f927a06a
parent_sha=8be6c2a3892a62d89ed17df3f3d3b54e9e84fda8ef6be2bcdaa7d49044593990
candidate_sha=c671ae022251a9a0f7d17cc851db7506d825c34854c69adedc6475d3da0f389f
parent_features_sha=82f8c1c3f217e45d3e02b60776bad5ec8268b8270a608990906802c38c8ce139
candidate_features_sha=8366b40e3b1951eda5ae2319119865423760c00c08960ba3b4de772c9caf82a8
added_feature_sha=ef3dd040cc4be53129ef57f369eb7bceaf46e51fa5ddd2c77b1677bbdf930fd8
audited_negative_tests_sha=709b3f86b0820c524cdd645a2993e7e17ae65f840936d388b9d7c890c2970412
execution_sha=e26ec9bb60b6289635c1ab1347a0e7c7372cc5c329998c9c1504299da452acd8
universe_sha=4ab5a2feb62b100afd4aa5e6afd9f418c415142a1678d1c2c1c9de9d1553c630
universe_keys_sha=46c6aea579898a22ab416c3d6c68b696ecb95fa5d4d8a3e19b4e068a1e0d3610
activation_sha=bdecdca8dbf0517221fbb8e403cacb0467add39c879b5659f71a7a838912fd74
activation_keys_sha=79ebef1024667e98fcb3ef3bfa4b11ec42cd142693d5bb6a60cb82136b985916
create_realm_sha=dc1778245ae4947fd29c7b2deb89c647ed57dd3bd85b84729ca4e7ea130a09a0
create_realm_keys_sha=33ec8c5bb476ae2de0c9031ac7ac2b020c107c2ea5276008f9a1bc37e9ccf0f0
all_keys_sha=69f0826f8f362d15c99b47e0fdd0aeb7dba2693f67abb255546f25cda026c797
historical_full_tsv_sha=c919dd56fc37f2946d729ee9a9a6958fc91c3f95366843ffae258953145e5a4f
historical_full_json_sha=342c22edd7cfdc4edf2b5085455c8586095bb4abc5b59d55cc4657c5ff954459
historical_parent_tsv_sha=6bfffe4686469f4e129aba88bf18b0ac13d1c97ef9d31d8afc013c03dd33e917
historical_parent_json_sha=a3a354d2e1bd8bc030e7f4e96db94cfea5e33966c4481674639e82b4d16d3216
formal_transition_sha=1616642079565934fe16ed488ede6f2017751700d9394f409e8e098534c507ab
formal_transition_data_sha=d349016f4b72388e1324c32bee65b845538d524f276ed46cf177f155343649ee
profile_transition_sha=760951d29b0f9b2b253ede34f03a0eec87b9f77988931c6784c678535f653a24
profile_transition_data_sha=99102ac67e1846a78d9053217e169d6c78c1e67227a1ad2be64269bbe9eb9a5c
baseline_lines=103
baseline_sha=28d20acc469f482e0fb139db9b615f15bf5b2a1e93b16fd5c260627ecfe9a0ff

usage() {
    printf 'usage: %s [--check|--full]\n' "${0##*/}"
    printf '  --check  verify frozen profiles, receipts, and scoped oracle inputs\n'
    printf '  --full   additionally reproduce both exact 102037-row profiles\n'
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
value() {
    awk -F= -v wanted="$1" \
        '$1==wanted{sub(/^[^=]*=/,"");print;found++} END{if(found!=1)exit 1}' \
        "$baseline"
}
canonical_value() {
    awk -F= -v wanted="$1" \
        '$1==wanted{sub(/^[^=]*=/,"");print;found++} END{if(found!=1)exit 1}' \
        "$canonical_baseline"
}
header() {
    awk -F= -v wanted="# $2" \
        '$1==wanted{sub(/^[^=]*=/,"");print;found++} END{if(found!=1)exit 1}' \
        "$1"
}
section() {
    awk -v wanted="[$2]" \
        '$0==wanted{inside=1;next} /^\[/{inside=0} inside&&NF&&$1!~/^#/{print}' \
        "$1"
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
check_file() {
    [[ -f "$1" && "$(lines "$1")" == "$2" && "$(sha "$1")" == "$3" ]] \
        || die "authenticated input drifted: $1"
}
report_rows() { awk -F'\t' '!/^#/&&!($1=="path"&&$2=="variant")' "$1"; }
report_keys() { report_rows "$1" | awk -F'\t' '{print $1 "\t" $2}' | sort; }
report_summary() { tail -n 1 "$1" | sed 's/^# summary //'; }
computed_summary() {
    report_rows "$1" | awk -F'\t' '{print $7}' | sort | uniq -c | awk '
        {out=out (NR==1?"":" ") $2 "=" $1} END{print out}'
}
report_count() {
    report_rows "$2" | awk -F'\t' -v wanted="$1" '$7==wanted{n++} END{print n+0}'
}
report_runnable() {
    report_rows "$1" | awk -F'\t' '$8!="selection"{n++} END{print n+0}'
}

json_result_projection() {
    awk -v report="$1" '
        function fail(message){
            printf "error: host-gc global JSONL %s: %s\n",report,message >"/dev/stderr"
            failed=1;exit 2
        }
        function expect(token){
            if(substr(line,position,length(token))!=token)fail("expected " token)
            position+=length(token)
        }
        function string_value(    character,escape,digits,result){
            expect("\"");result=""
            while(position<=length(line)){
                character=substr(line,position,1)
                if(character=="\""){position++;return result}
                if(character=="\\"){
                    position++;escape=substr(line,position,1)
                    if(escape=="u"){
                        digits=substr(line,position+1,4)
                        if(length(digits)!=4||digits~/[^0123456789abcdefABCDEF]/)fail("invalid Unicode escape")
                        result=result "\\u" digits;position+=5
                    }else{
                        if(index("\"\\/bfnrt",escape)==0)fail("invalid escape")
                        if(escape=="\"")result=result "\""
                        else if(escape=="/")result=result "/"
                        else if(escape=="b")result=result "\\u0008"
                        else if(escape=="f")result=result "\\u000c"
                        else result=result "\\" escape
                        position++
                    }
                    continue
                }
                if(character=="\t"||character=="\r")fail("unescaped control")
                result=result character;position++
            }
            fail("unterminated string")
        }
        function project(    i,key,result){
            line=$0;position=1;expect("{")
            for(i=1;i<=11;i++){
                if(i!=1)expect(",")
                key=string_value();if(key!=name[i])fail("unexpected field " key)
                expect(":");result=string_value()
                if(i==1){if(result!="result")fail("unexpected kind")}
                else field[i-1]=result
            }
            expect("}");if(position!=length(line)+1)fail("trailing data")
            print field[1],field[2],field[3],field[4],field[5],field[6],field[7],field[8],field[9],field[10]
        }
        BEGIN{
            OFS="\t";name[1]="kind";name[2]="path";name[3]="variant"
            name[4]="flags";name[5]="features";name[6]="expected_phase"
            name[7]="expected_type";name[8]="outcome";name[9]="actual_phase"
            name[10]="actual_type";name[11]="detail"
        }
        /^\{"kind":"metadata",/{next}
        /^\{"kind":"result",/{project();next}
        /^\{"kind":"summary",/{next}
        {fail("unexpected record")}
    ' "$1"
}
json_summary() {
    tail -n 1 "$1" | awk '
        /^\{"kind":"summary","outcomes":\{.*\}\}$/ {
            sub(/^\{"kind":"summary","outcomes":\{/,"");sub(/\}\}$/,"")
            gsub(/":/,"=");gsub(/"/,"");gsub(/,/," ");print;found++
        } END{if(found!=1)exit 1}'
}
expected_json_metadata() {
    printf '{"kind":"metadata","schema":2,"quickjs":"%s","test262":"%s","test262_patch_sha256":"%s","test262_config_sha256":"%s","test262_metadata_sha256":"%s","oxide_profile_sha256":"%s","profile":"test262-canonical-classified-v2","mode":"both"}\n' \
        "$quickjs" "$test262" "$patch_sha" "$config_sha" "$metadata_sha" "$1"
}
verify_report() {
    local tsv=$1 profile_sha=$2 rows=$3 keys_sha=$4 tsv_sha=$5 json_sha=$6
    local json=${tsv%.tsv}.jsonl projection=$tmp/projection.$$.tsv
    [[ -f "$tsv" && -f "$json" \
        && "$(header "$tsv" quickjs)" == "$quickjs" \
        && "$(header "$tsv" test262)" == "$test262" \
        && "$(header "$tsv" test262_patch_sha256)" == "$patch_sha" \
        && "$(header "$tsv" test262_config_sha256)" == "$config_sha" \
        && "$(header "$tsv" test262_metadata_sha256)" == "$metadata_sha" \
        && "$(header "$tsv" oxide_profile_sha256)" == "$profile_sha" \
        && "$(header "$tsv" profile)" == test262-canonical-classified-v2 \
        && "$(header "$tsv" mode)" == both \
        && "$(report_rows "$tsv" | wc -l | tr -d '[:space:]')" == "$rows" \
        && "$(report_keys "$tsv" | sha /dev/stdin)" == "$keys_sha" \
        && "$(sha "$tsv")" == "$tsv_sha" && "$(sha "$json")" == "$json_sha" \
        && "$(report_summary "$tsv")" == "$(computed_summary "$tsv")" \
        && "$(head -n 1 "$json")" == "$(expected_json_metadata "$profile_sha")" \
        && "$(json_summary "$json")" == "$(report_summary "$tsv")" ]] \
        || die "report identity drifted: $tsv"
    json_result_projection "$json" >"$projection" \
        || die "JSONL projection failed: $json"
    diff -u <(report_rows "$tsv") "$projection" \
        || die "JSONL/TSV projection drifted: $json"
}

check_profiles() {
    check_file "$parent" 1271 "$parent_sha"
    check_file "$candidate" 1272 "$candidate_sha"
    check_file "$live_profile" 1272 "$candidate_sha"
    cmp -s "$candidate" "$live_profile" \
        || die 'live Test262 profile is not byte-identical to host-gc candidate'
    pfeatures=$tmp/parent.features
    cfeatures=$tmp/candidate.features
    section "$parent" features >"$pfeatures"
    section "$candidate" features >"$cfeatures"
    [[ "$(lines "$pfeatures")" == 101 && "$(sha "$pfeatures")" == "$parent_features_sha" \
        && "$(lines "$cfeatures")" == 102 && "$(sha "$cfeatures")" == "$candidate_features_sha" \
        && "$(comm -13 "$pfeatures" "$cfeatures")" == host-gc-required \
        && "$(printf 'host-gc-required\n' | sha /dev/stdin)" == "$added_feature_sha" \
        && -z "$(comm -23 "$pfeatures" "$cfeatures")" ]] \
        || die 'global host-gc profile feature delta drifted'
    for name in audited-negative-tests execution; do
        section "$parent" "$name" >"$tmp/parent.$name"
        section "$candidate" "$name" >"$tmp/candidate.$name"
        diff -u "$tmp/parent.$name" "$tmp/candidate.$name"
    done
    [[ "$(lines "$tmp/parent.audited-negative-tests")" == 1157 \
        && "$(sha "$tmp/parent.audited-negative-tests")" == "$audited_negative_tests_sha" \
        && "$(lines "$tmp/parent.execution")" == 1 \
        && "$(sha "$tmp/parent.execution")" == "$execution_sha" ]] \
        || die 'global host-gc non-feature profile sections drifted'
}

check_inputs() {
    check_file "$baseline" "$baseline_lines" "$baseline_sha"
    check_file "$universe" 15 "$universe_sha"
    check_file "$activation" 14 "$activation_sha"
    check_file "$create_realm" 1 "$create_realm_sha"
    check_file "$historical_parent" 39 "$historical_parent_tsv_sha"
    check_file "$historical_parent_json" 30 "$historical_parent_json_sha"
    check_file "$transition" 33 "$formal_transition_sha"
    [[ "$(value historical_full_tsv_sha256)" == "$historical_full_tsv_sha" \
        && "$(value historical_full_jsonl_sha256)" == "$historical_full_json_sha" \
        && "$(value candidate_oxide_profile_sha256)" == "$candidate_sha" \
        && "$(value formal_transition_sha256)" == "$formal_transition_sha" \
        && "$(value profile_transition_sha256)" == "$profile_transition_sha" ]] \
        || die 'global host-gc baseline identity drifted'
    check_profiles
    [[ "$(toml_test262_value "$upstream" repository)" == https://github.com/tc39/test262.git \
        && "$(toml_test262_value "$upstream" commit)" == "$test262" \
        && "$(toml_test262_value "$upstream" patch_sha256)" == "$patch_sha" \
        && "$(toml_test262_value "$upstream" config_sha256)" == "$config_sha" \
        && "$(toml_test262_value "$upstream" test_count)" == 53125 \
        && "$(toml_test262_value "$upstream" metadata_records_sha256)" == "$metadata_sha" \
        && "$(toml_test262_value "$upstream" oxide_profile)" == "$live_profile" \
        && "$(toml_test262_value "$upstream" oxide_profile_sha256)" == "$candidate_sha" ]] \
        || die 'compat/upstream.toml host-gc admission binding drifted'
    [[ "$(canonical_value schema)" == test262-canonical-classified-v2 \
        && "$(canonical_value timeout_ms)" == 30000 \
        && "$(canonical_value variants)" == 102037 \
        && "$(canonical_value runnable)" == "$(value candidate_full_runnable)" \
        && "$(canonical_value passes)" == "$(value candidate_full_passes)" \
        && "$(canonical_value tsv_sha256)" == "$(value candidate_full_tsv_sha256)" \
        && "$(canonical_value jsonl_sha256)" == "$(value candidate_full_jsonl_sha256)" \
        && "$(canonical_value summary)" == "$(value candidate_full_summary)" ]] \
        || die 'canonical Test262 baseline does not identify R3ch candidate output'
}

run_report() {
    local profile=$1 output=$2 scope=$3 run_workers=$4
    args=(--suite "$suite" --config "$source_dir/test262.conf" \
        --oxide-profile "$profile" --report "$output" --mode both \
        --timeout-ms 30000 --workers "$run_workers" --allow-failures)
    if [[ "$scope" == full ]]; then args+=(--all); else args+=(--manifest "$universe"); fi
    "$runner" "${args[@]}" >/dev/null
}
make_transition() {
    local before=$1 after=$2 output=$3 title=$4
    {
        printf '# %s\n' "$title"
        echo "# before_oxide_profile_sha256=$parent_sha"
        echo "# after_oxide_profile_sha256=$candidate_sha"
        echo "# manifest_sha256=$universe_sha"
        printf 'path\tvariant\tflags\tfeatures\texpected_phase\texpected_type\tbefore_outcome\tbefore_actual_phase\tbefore_actual_type\tbefore_detail\tafter_outcome\tafter_actual_phase\tafter_actual_type\tafter_detail\n'
        awk -F'\t' 'BEGIN{OFS="\t"}
            NR==FNR{if(!/^#/&&!($1=="path"&&$2=="variant"))old[$1 FS $2]=$0;next}
            !/^#/&&!($1=="path"&&$2=="variant"){
                split(old[$1 FS $2],a,FS)
                print $1,$2,$3,$4,$5,$6,a[7],a[8],a[9],a[10],$7,$8,$9,$10
            }' "$before" "$after"
    } >"$output"
}

cd -- "$root"
tmp=$(mktemp -d "${TMPDIR:-/tmp}/quickjs-oxide-host-gc-global.XXXXXX")
trap 'rm -rf -- "$tmp"' EXIT HUP INT TERM
check_inputs
"$script_dir/test-test262-host-gc.sh" --check
if [[ "$mode" == check ]]; then
    check_inputs
    echo 'Global host-gc admission inputs verified: parent 101 features, candidate 102, QuickJS 28/28.'
    exit 0
fi

cargo build --locked --release --quiet --bin run-test262
runner=$root/target/release/run-test262
suite=$("$script_dir/prepare-test262.sh")
source_dir=$(dirname -- "$suite")
run_report "$parent" "$parent_report" focused "$workers"
run_report "$candidate" "$candidate_report" focused "$workers"
verify_report "$parent_report" "$parent_sha" 28 "$universe_keys_sha" \
    "$(value runtime_parent_focused_tsv_sha256)" \
    "$(value runtime_parent_focused_jsonl_sha256)"
verify_report "$candidate_report" "$candidate_sha" 28 "$universe_keys_sha" \
    "$(value candidate_focused_tsv_sha256)" \
    "$(value candidate_focused_jsonl_sha256)"
[[ "$(report_summary "$parent_report")" \
        == 'unsupported-feature=26 unsupported-host-create-realm=2' \
    && "$(report_summary "$candidate_report")" \
        == 'pass=26 unsupported-host-create-realm=2' ]] \
    || die 'global host-gc focused summaries drifted'

formal=$tmp/formal.tsv
profile_only=$tmp/profile.tsv
make_transition "$historical_parent" "$candidate_report" "$formal" \
    'R3ch global $262.gc host-hook admission transition.'
make_transition "$parent_report" "$candidate_report" "$profile_only" \
    'R3ch global $262.gc profile-only transition.'
[[ "$(sha "$formal")" == "$formal_transition_sha" \
    && "$(report_rows "$formal" | sha /dev/stdin)" == "$formal_transition_data_sha" \
    && "$(sha "$profile_only")" == "$profile_transition_sha" \
    && "$(report_rows "$profile_only" | sha /dev/stdin)" == "$profile_transition_data_sha" ]] \
    || die 'global host-gc focused transition receipt drifted'
diff -u "$transition" "$formal"
formal_counts=$(awk -F'\t' '!/^#/&&!($1=="path"&&$2=="variant"){
    different=0;for(i=7;i<=10;i++)if($i!=$(i+4))different=1
    if(different){changed++;if($7!=$11)outcome++;else detail++}else unchanged++
} END{printf "changed=%d outcome=%d detail=%d unchanged=%d",changed,outcome,detail,unchanged}' "$formal")
profile_counts=$(awk -F'\t' '!/^#/&&!($1=="path"&&$2=="variant"){
    different=0;for(i=7;i<=10;i++)if($i!=$(i+4))different=1
    if(different){changed++;if($7!=$11)outcome++;else detail++}else unchanged++
} END{printf "changed=%d outcome=%d detail=%d unchanged=%d",changed,outcome,detail,unchanged}' "$profile_only")
[[ "$formal_counts" == 'changed=28 outcome=26 detail=2 unchanged=0' \
    && "$profile_counts" == 'changed=26 outcome=26 detail=0 unchanged=2' ]] \
    || die "global host-gc focused join drifted: $formal_counts / $profile_counts"

if [[ "$mode" != full ]]; then
    check_inputs
    echo 'Global host-gc focused gate passes: historical 28-row delta is 26 pass + 2 detail-only; profile-only delta is 26 pass + 2 unchanged.'
    exit 0
fi

if [[ "$reuse_full_reports" == false ]]; then
    run_report "$parent" "$parent_full" full "$full_workers"
    run_report "$candidate" "$candidate_full" full "$full_workers"
fi
verify_report "$parent_full" "$parent_sha" 102037 "$all_keys_sha" \
    "$(value runtime_parent_full_tsv_sha256)" \
    "$(value runtime_parent_full_jsonl_sha256)"
verify_report "$candidate_full" "$candidate_sha" 102037 "$all_keys_sha" \
    "$(value candidate_full_tsv_sha256)" \
    "$(value candidate_full_jsonl_sha256)"
[[ "$(report_summary "$parent_full")" == "$(value runtime_parent_full_summary)" \
    && "$(report_summary "$candidate_full")" == "$(value candidate_full_summary)" \
    && "$(report_runnable "$parent_full")" == "$(value runtime_parent_full_runnable)" \
    && "$(report_runnable "$candidate_full")" == "$(value candidate_full_runnable)" \
    && "$(report_count pass "$parent_full")" == "$(value runtime_parent_full_passes)" \
    && "$(report_count pass "$candidate_full")" == "$(value candidate_full_passes)" ]] \
    || die 'global host-gc full receipt semantics drifted'

report_rows "$parent_report" >"$tmp/parent.focused"
report_rows "$candidate_report" >"$tmp/candidate.focused"
awk -F'\t' 'NR==FNR{wanted[$0]=1;next}!/^#/&&!($1=="path"&&$2=="variant")&&($1 in wanted)' \
    "$universe" "$parent_full" >"$tmp/parent.full-focused"
awk -F'\t' 'NR==FNR{wanted[$0]=1;next}!/^#/&&!($1=="path"&&$2=="variant")&&($1 in wanted)' \
    "$universe" "$candidate_full" >"$tmp/candidate.full-focused"
diff -u "$tmp/parent.focused" "$tmp/parent.full-focused"
diff -u "$tmp/candidate.focused" "$tmp/candidate.full-focused"

join_counts=$(awk -F'\t' -v parent="$parent_full" '
    FILENAME==parent{if(!/^#/&&!($1=="path"&&$2=="variant")){old[$1 FS $2]=$0;before++}next}
    !/^#/&&!($1=="path"&&$2=="variant"){
        key=$1 FS $2;if(!(key in old))exit 2;split(old[key],a,FS)
        for(i=1;i<=6;i++)if(a[i]!=$i)exit 3
        different=old[key]!=$0;if(a[7]=="pass"&&$7!="pass")regress++
        if(different){
            if($1=="test/staging/sm/extensions/dataview.js")exit 4
            changed++;if(a[7]!=$7)outcome++;else detail++
        }
        seen[key]=1
    }
    END{for(key in old)if(!(key in seen))exit 5
        printf "changed=%d outcome=%d detail=%d unchanged=%d regressions=%d",changed,outcome,detail,before-changed,regress}
' "$parent_full" "$candidate_full") || die 'global host-gc full join semantics drifted'
[[ "$join_counts" == 'changed=26 outcome=26 detail=0 unchanged=102011 regressions=0' ]] \
    || die "global host-gc full no-regression join drifted: $join_counts"
[[ "$(value historical_to_candidate_changed)" == 28 \
    && "$(value historical_to_candidate_outcome_changes)" == 26 \
    && "$(value historical_to_candidate_detail_changes)" == 2 \
    && "$(value historical_to_candidate_unchanged)" == 102009 \
    && "$(value historical_to_candidate_pass_regressions)" == 0 ]] \
    || die 'historical canonical full join receipt drifted'
check_inputs
echo 'Global host-gc full gate passes: 102037 rows, 26 profile changes, 26 new passes, 2 createRealm rows retained, zero pass regression.'
