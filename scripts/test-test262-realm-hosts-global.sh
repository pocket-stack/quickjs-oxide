#!/usr/bin/env bash
# Reproduce the R3ci global admission of $262.createRealm and $262.evalScript.

set -euo pipefail
export LC_ALL=C
export TZ=America/Los_Angeles

script_dir=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd)
root=$(CDPATH='' cd -- "$script_dir/.." && pwd)
baseline=tests/test262-realm-hosts-global-baseline.txt
canonical_baseline=tests/test262-full-baseline.txt
successor_baseline=tests/test262-binary-data-global-baseline.txt
parent=tests/test262-realm-hosts-global-parent.conf
candidate=tests/test262-realm-hosts-global-candidate.conf
successor_parent=tests/test262-binary-data-global-parent.conf
successor_candidate=tests/test262-binary-data-global-candidate.conf
successor_gate=scripts/test-test262-binary-data-global.sh
runtime_successor_gate=scripts/test-test262-string-normalize.sh
live_profile=compat/test262-oxide.conf
upstream=compat/upstream.toml
added_features=tests/test262-realm-hosts-global-added-features.txt
universe=tests/test262-realm-hosts-global-universe.txt
activation=tests/test262-realm-hosts-global-activation.txt
create_universe=tests/test262-create-realm-universe.txt
create_activation=tests/test262-create-realm-activation.txt
create_reason=tests/test262-create-realm-reason-only.txt
create_config_excluded=tests/test262-create-realm-config-excluded.txt
create_config_skipped=tests/test262-create-realm-config-skipped-feature.txt
eval_universe=tests/test262-eval-script-universe.txt
eval_activation=tests/test262-eval-script-activation.txt
historical_parent=tests/test262-realm-hosts-global-historical-parent.tsv
transition=tests/test262-realm-hosts-global-transitions.tsv
parent_report=target/test262-realm-hosts-global-parent.tsv
candidate_report=target/test262-realm-hosts-global-candidate.tsv
parent_full=target/test262-realm-hosts-global-parent-full.tsv
candidate_full=target/test262-realm-hosts-global-candidate-full.tsv
workers=${TEST262_WORKERS:-8}
full_workers=${TEST262_FULL_WORKERS:-2}
reuse_full_reports=${TEST262_REUSE_FULL_REPORTS:-false}

candidate_sha=01f936b9f5e0b920f10119a73f7e8ea52450863f113fff6542f3f241ed914d75
successor_sha=1e39c157e444f60f0a44f4fd373ad63147d814986cde5f08c4f5b33d8f5839a2
baseline_lines=121
baseline_sha=04a27c431883633e93cbe4abdd6eb19683ca1dce58050ab9e38365437d5fb472
successor_baseline_lines=95
successor_baseline_sha=0d188b3b2c0f65417e02e4c3350077f94665cb6de6e7ac4fc453750e1cfa83d6

usage() {
    printf 'usage: %s [--check|--full]\n' "${0##*/}"
    printf '  --check  verify frozen profiles, manifests, and receipts\n'
    printf '  --full   additionally replay and join both 102037-row profiles\n'
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
successor_value() {
    awk -F= -v wanted="$1" \
        '$1==wanted{sub(/^[^=]*=/,"");print;found++} END{if(found!=1)exit 1}' \
        "$successor_baseline"
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
            key=substr($0,1,separator-1)
            gsub(/^[[:space:]]+|[[:space:]]+$/, "", key)
            if(key!=wanted)next
            answer=substr($0,separator+1)
            gsub(/^[[:space:]]+|[[:space:]]+$/, "", answer)
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
    report_rows "$2" | awk -F'\t' -v wanted="$1" \
        '$7==wanted{count++} END{print count+0}'
}
report_runnable() {
    report_rows "$1" | awk -F'\t' '$8!="selection"{count++} END{print count+0}'
}
verify_report() {
    local report=$1 profile_sha=$2 rows=$3 keys_sha=$4 label=$5
    local json=${report%.tsv}.jsonl
    [[ -f "$report" && -f "$json" \
        && "$(header "$report" quickjs)" == "$(value quickjs)" \
        && "$(header "$report" test262)" == "$(value test262)" \
        && "$(header "$report" test262_patch_sha256)" == "$(value test262_patch_sha256)" \
        && "$(header "$report" test262_config_sha256)" == "$(value test262_config_sha256)" \
        && "$(header "$report" test262_metadata_sha256)" == "$(value test262_metadata_sha256)" \
        && "$(header "$report" oxide_profile_sha256)" == "$profile_sha" \
        && "$(header "$report" profile)" == test262-canonical-classified-v2 \
        && "$(header "$report" mode)" == both \
        && "$(report_rows "$report" | wc -l | tr -d '[:space:]')" == "$rows" \
        && "$(report_keys "$report" | sha /dev/stdin)" == "$keys_sha" \
        && "$(sha "$report")" == "$(value "${label}_tsv_sha256")" \
        && "$(sha "$json")" == "$(value "${label}_jsonl_sha256")" \
        && "$(report_summary "$report")" == "$(computed_summary "$report")" \
        && "$(report_summary "$report")" == "$(value "${label}_summary")" ]] \
        || die "classified report receipt drifted: $report"
}
variant_keys() {
    awk -F'\t' '
        function has(list,value){return index("," list ",", "," value ",")!=0}
        NR==FNR{wanted[$0]=1;next}
        $1 in wanted{
            if(has($3,"module")||has($3,"noStrict")||has($3,"raw"))print $1 "\tsloppy"
            else if(has($3,"onlyStrict"))print $1 "\tstrict"
            else{print $1 "\tsloppy";print $1 "\tstrict"}
        }
    ' "$1" "$metadata_tsv" | sort
}
make_transition() {
    local before=$1 after=$2 output=$3 title=$4
    {
        printf '# %s\n' "$title"
        echo "# before_oxide_profile_sha256=$(value runtime_parent_oxide_profile_sha256)"
        echo "# after_oxide_profile_sha256=$(value candidate_oxide_profile_sha256)"
        echo "# manifest_sha256=$(value universe_sha256)"
        printf 'path\tvariant\tflags\tfeatures\texpected_phase\texpected_type\tbefore_outcome\tbefore_actual_phase\tbefore_actual_type\tbefore_detail\tafter_outcome\tafter_actual_phase\tafter_actual_type\tafter_detail\n'
        awk -F'\t' 'BEGIN{OFS="\t"}
            NR==FNR{if(!/^#/&&!($1=="path"&&$2=="variant"))old[$1 FS $2]=$0;next}
            !/^#/&&!($1=="path"&&$2=="variant"){
                split(old[$1 FS $2],a,FS)
                print $1,$2,$3,$4,$5,$6,a[7],a[8],a[9],a[10],$7,$8,$9,$10
            }' "$before" "$after"
    } >"$output"
}
transition_counts() {
    awk -F'\t' '!/^#/&&!($1=="path"&&$2=="variant"){
        different=0;for(i=7;i<=10;i++)if($i!=$(i+4))different=1
        if(different){changed++;if($7!=$11)outcome++;else detail++}else unchanged++
    } END{printf "changed=%d outcome=%d detail=%d unchanged=%d",changed,outcome,detail,unchanged}' "$1"
}

bridge_r3cj_successor() {
    [[ -f "$live_profile" ]] || return 0
    check_file "$baseline" "$baseline_lines" "$baseline_sha"
    check_file "$parent" 1272 "$(value runtime_parent_oxide_profile_sha256)"
    check_file "$candidate" 1274 "$candidate_sha"
    check_file "$added_features" 2 "$(value added_features_sha256)"
    check_file "$universe" 312 "$(value universe_sha256)"
    check_file "$activation" 110 "$(value activation_sha256)"
    check_file "$historical_parent" 600 "$(value historical_parent_focused_tsv_sha256)"
    check_file "$transition" 594 "$(value formal_transition_sha256)"
    check_file "$successor_baseline" "$successor_baseline_lines" "$successor_baseline_sha"
    check_file "$successor_parent" 1274 "$candidate_sha"
    check_file "$successor_candidate" 1292 "$successor_sha"
    cmp -s "$candidate" "$successor_parent" \
        || die 'R3ci candidate is not byte-identical to the R3cj parent'
    [[ "$(value candidate_oxide_profile_sha256)" == "$candidate_sha" \
        && "$(successor_value parent_oxide_profile_sha256)" == "$candidate_sha" \
        && "$(successor_value candidate_oxide_profile_sha256)" == "$successor_sha" \
        && "$(successor_value parent_full_tsv_sha256)" == "$(value candidate_full_tsv_sha256)" \
        && "$(successor_value parent_full_jsonl_sha256)" == "$(value candidate_full_jsonl_sha256)" \
        && "$(successor_value parent_full_summary)" == "$(value candidate_full_summary)" \
        && "$(successor_value full_changed)" == 396 \
        && "$(successor_value full_outcome_changed)" == 386 \
        && "$(successor_value full_detail_only)" == 10 \
        && "$(successor_value full_pass_regressions)" == 0 ]] \
        || die 'R3cj successor does not checksum-bridge the historical R3ci receipt'
    if [[ "$(sha "$live_profile")" != "$successor_sha" \
        || "$(canonical_value tsv_sha256)" \
            != "$(successor_value candidate_full_tsv_sha256)" ]]; then
        case $mode in
            check) "$runtime_successor_gate" --check ;;
            focused) "$runtime_successor_gate" ;;
            full) "$runtime_successor_gate" --full ;;
        esac
        echo 'Historical R3ci realm-host receipt is transitively checksum-bridged through the runtime successor chain.'
        exit 0
    fi
    check_file "$live_profile" 1292 "$successor_sha"
    cmp -s "$successor_candidate" "$live_profile" \
        || die 'live Test262 profile is not byte-identical to the R3cj candidate'
    [[ "$(canonical_value runnable)" == "$(successor_value candidate_full_runnable)" \
        && "$(canonical_value passes)" == "$(successor_value candidate_full_passes)" \
        && "$(canonical_value tsv_sha256)" == "$(successor_value candidate_full_tsv_sha256)" \
        && "$(canonical_value jsonl_sha256)" == "$(successor_value candidate_full_jsonl_sha256)" \
        && "$(canonical_value summary)" == "$(successor_value candidate_full_summary)" \
        && "$(toml_test262_value "$upstream" oxide_profile_sha256)" == "$successor_sha" ]] \
        || die 'R3cj successor does not checksum-bridge the historical R3ci receipt'
    case $mode in
        check) "$successor_gate" --check ;;
        focused) "$successor_gate" ;;
        full) "$successor_gate" --full ;;
    esac
    echo 'Historical R3ci realm-host receipt is checksum-bridged through the replayed R3cj successor.'
    exit 0
}

check_profiles() {
    check_file "$parent" 1272 "$(value runtime_parent_oxide_profile_sha256)"
    check_file "$candidate" 1274 "$(value candidate_oxide_profile_sha256)"
    check_file "$live_profile" 1274 "$(value candidate_oxide_profile_sha256)"
    cmp -s "$candidate" "$live_profile" \
        || die 'live Test262 profile is not byte-identical to the R3ci candidate'
    pfeatures=$tmp/parent.features
    cfeatures=$tmp/candidate.features
    section "$parent" features >"$pfeatures"
    section "$candidate" features >"$cfeatures"
    [[ "$(lines "$pfeatures")" == 102 \
        && "$(sha "$pfeatures")" == "$(value parent_features_sha256)" \
        && "$(lines "$cfeatures")" == 104 \
        && "$(sha "$cfeatures")" == "$(value candidate_features_sha256)" ]] \
        || die 'realm-host global feature inventories drifted'
    diff -u "$added_features" <(comm -13 "$pfeatures" "$cfeatures")
    [[ -z "$(comm -23 "$pfeatures" "$cfeatures")" ]] \
        || die 'realm-host candidate removed a parent feature'
    for name in audited-negative-tests execution; do
        section "$parent" "$name" >"$tmp/parent.$name"
        section "$candidate" "$name" >"$tmp/candidate.$name"
        diff -u "$tmp/parent.$name" "$tmp/candidate.$name"
    done
    [[ "$(lines "$tmp/parent.audited-negative-tests")" == 1157 \
        && "$(sha "$tmp/parent.audited-negative-tests")" \
            == "$(value audited_negative_tests_sha256)" \
        && "$(lines "$tmp/parent.execution")" == 1 \
        && "$(sha "$tmp/parent.execution")" == "$(value execution_sha256)" ]] \
        || die 'realm-host non-feature profile sections drifted'
}

check_manifests() {
    for file in "$universe" "$activation" "$added_features" "$create_universe" \
            "$create_activation" "$create_reason" "$create_config_excluded" \
            "$create_config_skipped" "$eval_universe" "$eval_activation"; do
        sort -c "$file" || die "manifest is not bytewise sorted: $file"
        [[ -z "$(uniq -d "$file")" ]] || die "manifest contains duplicates: $file"
    done
    diff -u "$universe" <(sort -u "$create_universe" "$eval_universe")
    [[ -z "$(comm -12 "$create_universe" "$eval_universe")" ]] \
        || die 'createRealm and evalScript source universes overlap'
    diff -u "$activation" <(sort -u "$create_activation" "$eval_activation")
    [[ -z "$(comm -12 "$create_activation" "$eval_activation")" ]] \
        || die 'createRealm and evalScript activation cohorts overlap'
    cat "$create_activation" "$create_reason" "$create_config_excluded" \
        "$create_config_skipped" | sort >"$tmp/create.partition"
    diff -u "$create_universe" "$tmp/create.partition"
    [[ -z "$(uniq -d "$tmp/create.partition")" ]] \
        || die 'createRealm global partition overlaps'
}

check_inputs() {
    check_file "$added_features" 2 "$(value added_features_sha256)"
    check_file "$universe" 312 "$(value universe_sha256)"
    check_file "$activation" 110 "$(value activation_sha256)"
    check_file "$historical_parent" 600 "$(value historical_parent_focused_tsv_sha256)"
    check_file "$transition" 594 "$(value formal_transition_sha256)"
    check_file "$create_universe" 281 "$(value create_realm_universe_sha256)"
    check_file "$create_activation" 79 "$(value create_realm_activation_sha256)"
    check_file "$create_reason" 174 "$(value create_realm_reason_only_sha256)"
    check_file "$create_config_excluded" 11 "$(value create_realm_config_excluded_sha256)"
    check_file "$create_config_skipped" 17 "$(value create_realm_config_skipped_sha256)"
    check_file "$eval_universe" 31 "$(value eval_script_universe_sha256)"
    check_file "$eval_activation" 31 "$(value eval_script_activation_sha256)"
    check_profiles
    check_manifests
    [[ "$(header "$historical_parent" oxide_profile_sha256)" \
            == "$(value historical_parent_oxide_profile_sha256)" \
        && "$(report_keys "$historical_parent" | sha /dev/stdin)" \
            == "$(value universe_keys_sha256)" \
        && "$(report_summary "$historical_parent")" \
            == "$(value historical_parent_focused_summary)" \
        && "$(toml_test262_value "$upstream" repository)" \
            == https://github.com/tc39/test262.git \
        && "$(toml_test262_value "$upstream" commit)" == "$(value test262)" \
        && "$(toml_test262_value "$upstream" patch_sha256)" \
            == "$(value test262_patch_sha256)" \
        && "$(toml_test262_value "$upstream" config_sha256)" \
            == "$(value test262_config_sha256)" \
        && "$(toml_test262_value "$upstream" metadata_records_sha256)" \
            == "$(value test262_metadata_sha256)" \
        && "$(toml_test262_value "$upstream" oxide_profile)" == "$live_profile" \
        && "$(toml_test262_value "$upstream" oxide_profile_sha256)" \
            == "$(value candidate_oxide_profile_sha256)" ]] \
        || die 'realm-host upstream or historical binding drifted'
    [[ "$(canonical_value schema)" == test262-canonical-classified-v2 \
        && "$(canonical_value timeout_ms)" == 30000 \
        && "$(canonical_value variants)" == 102037 \
        && "$(canonical_value runnable)" == "$(value candidate_full_runnable)" \
        && "$(canonical_value passes)" == "$(value candidate_full_passes)" \
        && "$(canonical_value tsv_sha256)" == "$(value candidate_full_tsv_sha256)" \
        && "$(canonical_value jsonl_sha256)" == "$(value candidate_full_jsonl_sha256)" \
        && "$(canonical_value summary)" == "$(value candidate_full_summary)" ]] \
        || die 'canonical Test262 baseline does not identify the R3ci candidate'
}

run_report() {
    local profile=$1 output=$2 scope=$3 pool=$4
    local -a selected
    if [[ "$scope" == full ]]; then selected=(--all)
    else selected=(--manifest "$universe"); fi
    "$runner" --suite "$suite" --config "$source_dir/test262.conf" \
        --oxide-profile "$profile" "${selected[@]}" --report "$output" \
        --mode both --timeout-ms 30000 --workers "$pool" --allow-failures >/dev/null
}

cd -- "$root"
bridge_r3cj_successor
tmp=$(mktemp -d "${TMPDIR:-/tmp}/quickjs-oxide-realm-hosts-global.XXXXXX")
trap 'rm -rf -- "$tmp"' EXIT HUP INT TERM
check_inputs
if [[ "$mode" == check ]]; then
    echo 'R3ci realm-host global inputs verified: 312 paths, 589 variants, exact 102-to-104 feature delta.'
    exit 0
fi

cargo build --locked --release --quiet --bin run-test262
runner=$root/target/release/run-test262
suite=$("$script_dir/prepare-test262.sh")
source_dir=$(dirname -- "$suite")
metadata_bin=$tmp/metadata.bin
metadata_tsv=$tmp/metadata.tsv
"$runner" --suite "$suite" --validate-metadata "$metadata_bin" >/dev/null
[[ "$(sha "$metadata_bin")" == "$(value test262_metadata_sha256)" ]] \
    || die 'pinned Test262 metadata drifted'
tr '\0' '\t' <"$metadata_bin" >"$metadata_tsv"
variant_keys "$universe" >"$tmp/universe.keys"
variant_keys "$activation" >"$tmp/activation.keys"
[[ "$(lines "$tmp/universe.keys")" == 589 \
    && "$(sha "$tmp/universe.keys")" == "$(value universe_keys_sha256)" \
    && "$(lines "$tmp/activation.keys")" == 194 \
    && "$(sha "$tmp/activation.keys")" == "$(value activation_keys_sha256)" ]] \
    || die 'realm-host variant-key inventory drifted'

run_report "$parent" "$parent_report" focused "$workers"
run_report "$candidate" "$candidate_report" focused "$workers"
verify_report "$parent_report" "$(value runtime_parent_oxide_profile_sha256)" \
    589 "$(value universe_keys_sha256)" runtime_parent_focused
verify_report "$candidate_report" "$(value candidate_oxide_profile_sha256)" \
    589 "$(value universe_keys_sha256)" candidate_focused

formal=$tmp/formal.tsv
profile_only=$tmp/profile.tsv
make_transition "$historical_parent" "$candidate_report" "$formal" \
    'Exhaustive $262.createRealm/$262.evalScript global admission transition.'
make_transition "$parent_report" "$candidate_report" "$profile_only" \
    'Runtime-parent to candidate $262.createRealm/$262.evalScript global profile transition.'
diff -u "$transition" "$formal"
[[ "$(sha "$formal")" == "$(value formal_transition_sha256)" \
    && "$(report_rows "$formal" | sha /dev/stdin)" \
        == "$(value formal_transition_data_sha256)" \
    && "$(sha "$profile_only")" == "$(value profile_transition_sha256)" \
    && "$(report_rows "$profile_only" | sha /dev/stdin)" \
        == "$(value profile_transition_data_sha256)" \
    && "$(transition_counts "$formal")" \
        == 'changed=534 outcome=534 detail=0 unchanged=55' \
    && "$(transition_counts "$profile_only")" \
        == 'changed=534 outcome=194 detail=340 unchanged=55' ]] \
    || die 'realm-host focused transition semantics drifted'

if [[ "$mode" != full ]]; then
    check_inputs
    echo 'R3ci realm-host focused gate passes: 194 new passes, 340 reason-only changes, 55 unchanged selections.'
    exit 0
fi

if [[ "$reuse_full_reports" == false ]]; then
    run_report "$parent" "$parent_full" full "$full_workers"
    run_report "$candidate" "$candidate_full" full "$full_workers"
fi
verify_report "$parent_full" "$(value runtime_parent_oxide_profile_sha256)" \
    102037 "$(value full_keys_sha256)" runtime_parent_full
verify_report "$candidate_full" "$(value candidate_oxide_profile_sha256)" \
    102037 "$(value full_keys_sha256)" candidate_full
[[ "$(report_runnable "$parent_full")" == "$(value runtime_parent_full_runnable)" \
    && "$(report_count pass "$parent_full")" == "$(value runtime_parent_full_passes)" \
    && "$(report_runnable "$candidate_full")" == "$(value candidate_full_runnable)" \
    && "$(report_count pass "$candidate_full")" == "$(value candidate_full_passes)" ]] \
    || die 'realm-host full receipt semantics drifted'

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
        if(different){changed++;if(a[7]!=$7)outcome++;else detail++}
        seen[key]=1
    }
    END{for(key in old)if(!(key in seen))exit 4
        printf "changed=%d outcome=%d detail=%d unchanged=%d regressions=%d",changed,outcome,detail,before-changed,regress}
' "$parent_full" "$candidate_full") || die 'realm-host full join failed'
[[ "$join_counts" \
    == 'changed=534 outcome=194 detail=340 unchanged=101503 regressions=0' ]] \
    || die "realm-host full no-regression join drifted: $join_counts"
check_inputs
echo 'R3ci realm-host full gate passes: 102037 rows, 194 new passes, 340 reason-only changes, zero pass regressions.'
